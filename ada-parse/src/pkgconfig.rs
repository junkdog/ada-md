//! pkg-config's `.pc` files, and their templates (`.pc.in`, `.pc.cmake`), on the line-format
//! engine (X-146): ada's own reading, bat's set has no syntax for them. As pkgconf (3.0) reads a
//! file: a line is a variable (`name=value`), a field (`Key: value`) or a comment; a `#` starts a
//! comment wherever it stands (`Description: PKCS#11` is `PKCS` to pkgconf), `\#` is a `#`, `$$` a
//! `$`, `${name}` a variable's value, and a `\` that ends a line joins the next to it. A key is
//! read to its `=` or `:` past blanks (pkgconf warns of them, and reads on).
//!
//! A value is read by its field: `Version`'s a version; `Requires`', `Requires.private`'s,
//! `Conflicts`' and `Provides`' a list of modules, each with a comparison and a version; `Libs`',
//! `Cflags`' and their `.private`'s a command line, whose `-` words are options; `URL`'s a link;
//! the others' and a variable's text. Keys are matched ignoring ASCII case, as the files spell
//! them (`CFlags`, `Libs.Private`). A template's placeholder (`@VAR@`, CMake's and autoconf's) is
//! read wherever it stands, as `${…}` is; a line with neither `=` nor `:` (a template's
//! `@pc_data@`, which pkgconf would skip) has only its placeholders and `${…}` read.
//!
//! What a line leaves open is the value a `\` at its end continues: the next line is that value.

use crate::emit::{Push, Runs, Sink};
use crate::line_format::{Cursor, Format, blanks};
use crate::{Out, State};

const COMMENT: u16 = 0;
const COMMENT_MARK: u16 = 1;
const VARIABLE: u16 = 2;
const ASSIGNMENT: u16 = 3;
const FIELD: u16 = 4;
const SEPARATOR: u16 = 5;
const TEXT: u16 = 6;
const VERSION: u16 = 7;
const MODULE: u16 = 8;
const COMPARISON: u16 = 9;
const OPTION: u16 = 10;
const LINK: u16 = 11;
const SUBST_BEGIN: u16 = 12;
const SUBST_NAME: u16 = 13;
const SUBST_END: u16 = 14;
const ESCAPE: u16 = 15;
const PLACEHOLDER_MARK: u16 = 16;
const PLACEHOLDER: u16 = 17;

/// Each kind's scopes under `source.pkgconfig`: ada's own (no syntax of bat's set reads
/// pkg-config), the leaves other languages give these tokens, told apart by the themes.
pub const KINDS: &[&str] = &[
    "comment.line.number-sign.pkgconfig",
    "comment.line.number-sign.pkgconfig punctuation.definition.comment.pkgconfig",
    "entity.name.variable.pkgconfig",
    "keyword.operator.assignment.pkgconfig",
    "keyword.other.definition.pkgconfig",
    "punctuation.separator.key-value.pkgconfig",
    "string.unquoted.pkgconfig",
    "constant.numeric.version.pkgconfig",
    "entity.name.package.pkgconfig",
    "keyword.operator.comparison.pkgconfig",
    "variable.parameter.option.pkgconfig",
    "markup.underline.link.pkgconfig",
    "punctuation.definition.variable.begin.pkgconfig",
    "variable.other.readwrite.pkgconfig",
    "punctuation.definition.variable.end.pkgconfig",
    "constant.character.escape.pkgconfig",
    "punctuation.definition.placeholder.pkgconfig",
    "variable.other.placeholder.pkgconfig",
];

// What a value is read as, by its key; a line's open word is the value its `\` continues.
/// A variable's value, or a field's that is text (`Name`, `Description`, unknown keys).
const V_TEXT: u64 = 1;
const V_VERSION: u64 = 2;
/// `Requires`, `Requires.private`, `Conflicts`, `Provides`: modules, comparisons, versions.
const V_MODULES: u64 = 3;
/// `Libs`, `Cflags`, and their `.private`: options and their arguments.
const V_OPTIONS: u64 = 4;
const V_LINK: u64 = 5;
/// The text of a line without a key: only its substitutions and placeholders read.
const V_NONE: u64 = 6;

/// Lex `b`, a line without its `\n`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    crate::line_format::line::<PkgConfig, _>(b, state, out, &mut Push);
}

/// [`line`]'s lexemes written to `k`, a run sink.
#[inline(never)]
pub(crate) fn line_runs(b: &[u8], state: &mut State, out: &mut Out, k: &mut Runs) {
    crate::line_format::line::<PkgConfig, _>(b, state, out, k);
}

struct PkgConfig;

impl Format for PkgConfig {
    const LEAD: [u8; 256] = crate::line_format::leads(b"#", b"");
    const COMMENT_TEXT: u16 = COMMENT;
    const COMMENT_MARK: u16 = COMMENT_MARK;
    const COMMENT_BLANKS: bool = false;

    /// A value's next line, after a `\` that ended the one before.
    #[inline(always)]
    fn resume<K: Sink>(c: &mut Cursor<K>, open: u64) -> u64 {
        value(c, 0, open)
    }

    /// A variable or a field: its key (name characters and placeholders), blanks, its `=` or
    /// `:`, blanks, its value; a line without one is [`V_NONE`]'s.
    #[inline(always)]
    fn entry<K: Sink>(c: &mut Cursor<K>, p: usize) -> u64 {
        let b = c.b;
        let mut q = p;
        while q < b.len() {
            if KEY[usize::from(b[q])] {
                q += 1;
            } else if let Some(e) = placeholder(b, q) {
                q = e;
            } else {
                break;
            }
        }
        let r = blanks(b, q);
        let (op, sep) = match b.get(r) {
            Some(b'=') if q > p => (V_TEXT, ASSIGNMENT),
            Some(b':') if q > p => (field(&b[p..q]), SEPARATOR),
            _ => return value(c, p, V_NONE),
        };
        let name = if sep == ASSIGNMENT { VARIABLE } else { FIELD };
        key(c, p, q, name);
        c.skip(r);
        c.put(r + 1, sep);
        let v = blanks(b, r + 1);
        c.skip(v);
        value(c, v, op)
    }
}

/// A key's bytes: ASCII letters and digits, `_` and `.`, as pkgconf reads one.
static KEY: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'_' || b == b'.';
        c += 1;
    }
    t
};

/// A placeholder's name's bytes: CMake's `configure_file` (`[A-Za-z0-9_/.+-]`), which takes
/// autoconf's.
static PLACEHOLDER_NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'.' | b'+' | b'-');
        c += 1;
    }
    t
};

/// A template's placeholder at `p`, an `@`, a name and an `@` on the line: its end.
#[inline(always)]
fn placeholder(b: &[u8], p: usize) -> Option<usize> {
    if b[p] != b'@' {
        return None;
    }
    let mut e = p + 1;
    while e < b.len() && PLACEHOLDER_NAME[usize::from(b[e])] {
        e += 1;
    }
    (e > p + 1 && b.get(e) == Some(&b'@')).then_some(e + 1)
}

/// `@VAR@` at `p`, ending at `e`: its marks and its name. Out of line, as the value's other rare
/// tokens' writes are ([`tok`]): each of the run sink's writes inlined is a few hundred bytes.
#[inline(never)]
fn put_placeholder<K: Sink>(c: &mut Cursor<K>, p: usize, e: usize) {
    c.put(p + 1, PLACEHOLDER_MARK);
    c.put(e - 1, PLACEHOLDER);
    c.put(e, PLACEHOLDER_MARK);
}

/// A key `b[p..q]` as `kind`, its placeholders read.
#[inline(never)]
fn key<K: Sink>(c: &mut Cursor<K>, p: usize, q: usize, kind: u16) {
    c.skip(p);
    let mut s = p;
    while s < q {
        match placeholder(c.b, s) {
            Some(e) if e <= q => {
                put_placeholder(c, s, e);
                s = e;
            }
            _ => {
                let mut e = s + 1;
                while e < q && c.b[e] != b'@' {
                    e += 1;
                }
                c.put(e, kind);
                s = e;
            }
        }
    }
}

/// What a field's value is read as, by its key ignoring ASCII case.
fn field(k: &[u8]) -> u64 {
    let is = |w: &[u8]| k.eq_ignore_ascii_case(w);
    if is(b"Version") {
        V_VERSION
    } else if is(b"Requires") || is(b"Requires.private") || is(b"Conflicts") || is(b"Provides") {
        V_MODULES
    } else if is(b"Libs") || is(b"Libs.private") || is(b"Cflags") || is(b"Cflags.private") {
        V_OPTIONS
    } else if is(b"URL") {
        V_LINK
    } else {
        V_TEXT
    }
}

/// What a value's bytes are to its reader: text, or one of these.
const B_TEXT: u8 = 0;
const B_BLANK: u8 = 1;
const B_HASH: u8 = 2;
const B_BACKSLASH: u8 = 3;
const B_DOLLAR: u8 = 4;
const B_AT: u8 = 5;
/// `,` in a list of modules.
const B_COMMA: u8 = 6;
/// `<`, `>`, `=`, `!`: a comparison in a list of modules.
const B_COMPARE: u8 = 7;
static VALUE_BYTE: [u8; 256] = {
    let mut t = [B_TEXT; 256];
    t[b' ' as usize] = B_BLANK;
    t[b'\t' as usize] = B_BLANK;
    t[b'\r' as usize] = B_BLANK;
    t[b'#' as usize] = B_HASH;
    t[b'\\' as usize] = B_BACKSLASH;
    t[b'$' as usize] = B_DOLLAR;
    t[b'@' as usize] = B_AT;
    t[b',' as usize] = B_COMMA;
    t[b'<' as usize] = B_COMPARE;
    t[b'>' as usize] = B_COMPARE;
    t[b'=' as usize] = B_COMPARE;
    t[b'!' as usize] = B_COMPARE;
    t
};

/// The text before `s` as `text`, then `s..e` as `kind`: a call, for the rare tokens.
#[inline(never)]
fn tok<K: Sink>(c: &mut Cursor<K>, text: Option<u16>, s: usize, e: usize, kind: u16) {
    c.tok(text, s, e, kind);
}

/// `${name}` from `p` to its `}` at `close`, the text before it as `text`.
#[inline(never)]
fn subst<K: Sink>(c: &mut Cursor<K>, text: Option<u16>, p: usize, close: usize) {
    c.tok(text, p, p + 2, SUBST_BEGIN);
    c.put(close, SUBST_NAME);
    c.put(close + 1, SUBST_END);
}

/// A value read as `v` from `p` to the line's end: what the line leaves open (`v` where a `\`
/// ends it). One copy a sink, for a fresh line's value and a continued one (inlined in both,
/// with the sink's writes inlined at each token, it was 40 KB of ada).
#[inline(never)]
fn value<K: Sink>(c: &mut Cursor<K>, mut p: usize, v: u64) -> u64 {
    let b = c.b;
    let n = b.len();
    let list = v == V_MODULES || v == V_OPTIONS;
    // the kind of the word the text before `p` is in: a module's or its version, an option or
    // an argument, the value's text
    let mut word = match v {
        V_VERSION => Some(VERSION),
        V_LINK => Some(LINK),
        V_NONE => None,
        _ => Some(TEXT),
    };
    // in a list, the next word's first byte decides it; after a comparison, a version
    let mut fresh = true;
    let mut after_comparison = false;
    // no `}` past here: a `${` after that is text (each one searched again would be quadratic)
    let mut unclosed = false;
    while p < n {
        let x = b[p];
        let class = VALUE_BYTE[usize::from(x)];
        if list && fresh && class != B_BLANK {
            fresh = false;
            word = Some(match v {
                V_MODULES if after_comparison => VERSION,
                V_MODULES => MODULE,
                _ if x == b'-' => OPTION,
                _ => TEXT,
            });
            after_comparison = false;
        }
        match class {
            B_HASH => {
                c.text(word, p);
                c.comment::<PkgConfig>(p);
                return 0;
            }
            B_BACKSLASH if p + 1 == n => {
                tok(c, word, p, n, ESCAPE);
                return v;
            }
            B_BACKSLASH if b[p + 1] == b'#' => {
                tok(c, word, p, p + 2, ESCAPE);
                p += 2;
            }
            B_DOLLAR if b.get(p + 1) == Some(&b'$') => {
                tok(c, word, p, p + 2, ESCAPE);
                p += 2;
            }
            B_DOLLAR if !unclosed && b.get(p + 1) == Some(&b'{') => {
                match b[p + 2..].iter().position(|&y| y == b'}') {
                    Some(i) => {
                        let close = p + 2 + i;
                        subst(c, word, p, close);
                        p = close + 1;
                    }
                    None => {
                        unclosed = true;
                        p += 1;
                    }
                }
            }
            B_AT => match placeholder(b, p) {
                Some(e) => {
                    c.text(word, p);
                    put_placeholder(c, p, e);
                    p = e;
                }
                None => p += 1,
            },
            B_BLANK if list => {
                c.text(word, p);
                p = blanks(b, p);
                c.skip(p);
                fresh = true;
            }
            B_COMMA if v == V_MODULES => {
                c.text(word, p);
                p += 1;
                c.skip(p);
                fresh = true;
            }
            B_COMPARE if v == V_MODULES => {
                let mut e = p + 1;
                while e < n && VALUE_BYTE[usize::from(b[e])] == B_COMPARE {
                    e += 1;
                }
                tok(c, word, p, e, COMPARISON);
                p = e;
                fresh = true;
                after_comparison = true;
            }
            _ => p += 1,
        }
    }
    c.text(word, n);
    0
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn has(lines: &[&str], text: &str, scope: &str) {
        let got = marked(Lang::PkgConfig, lines);
        assert!(
            got.iter().any(|&(t, s)| t == text && s == scope),
            "{text:?} as {scope:?} in {got:#?}"
        );
    }

    #[test]
    fn variables_fields_and_comments() {
        let l = [
            "# a comment",
            "prefix=/usr",
            "libdir = ${prefix}/lib # where",
            "Name: zlib",
            "Description: PKCS#11 and \\#2",
            "Version: 1.3.1",
            "URL: https://zlib.net/",
            "CFlags: -I${includedir} -DX=1",
            "Libs.Private: -lm /usr/lib/a.so",
            "Requires: glib-2.0 >= 2.50, gio-2.0",
            "y=$${prefix}",
        ];
        has(&l, " a comment", "comment.line.number-sign.pkgconfig");
        has(&l, "prefix", "entity.name.variable.pkgconfig");
        has(&l, "=", "keyword.operator.assignment.pkgconfig");
        has(&l, "/usr", "string.unquoted.pkgconfig");
        // blanks before the `=`: pkgconf warns, and reads on
        has(&l, "libdir", "entity.name.variable.pkgconfig");
        has(&l, "${", "punctuation.definition.variable.begin.pkgconfig");
        has(&l, "prefix", "variable.other.readwrite.pkgconfig");
        has(&l, "/lib ", "string.unquoted.pkgconfig");
        has(&l, " where", "comment.line.number-sign.pkgconfig");
        has(&l, "Name", "keyword.other.definition.pkgconfig");
        has(&l, ":", "punctuation.separator.key-value.pkgconfig");
        has(&l, "zlib", "string.unquoted.pkgconfig");
        // a `#` anywhere is a comment, as pkgconf reads it; `\#` a `#`
        has(&l, "PKCS", "string.unquoted.pkgconfig");
        has(&l, "11 and \\#2", "comment.line.number-sign.pkgconfig");
        has(&l, "1.3.1", "constant.numeric.version.pkgconfig");
        has(&l, "https://zlib.net/", "markup.underline.link.pkgconfig");
        // keys ignoring case; options and their arguments
        has(&l, "-I", "variable.parameter.option.pkgconfig");
        has(&l, "-DX=1", "variable.parameter.option.pkgconfig");
        has(&l, "-lm", "variable.parameter.option.pkgconfig");
        has(&l, "/usr/lib/a.so", "string.unquoted.pkgconfig");
        has(&l, "glib-2.0", "entity.name.package.pkgconfig");
        has(&l, ">=", "keyword.operator.comparison.pkgconfig");
        has(&l, "2.50", "constant.numeric.version.pkgconfig");
        has(&l, "gio-2.0", "entity.name.package.pkgconfig");
        has(&l, "$$", "constant.character.escape.pkgconfig");
        has(&l, "{prefix}", "string.unquoted.pkgconfig");
    }

    #[test]
    fn escapes_and_continuations() {
        let l = [
            "x=a\\#b",
            "Cflags: -I/a \\",
            "  -DB",
            "Requires: m \\",
            "Libs: n",
        ];
        has(&l, "\\#", "constant.character.escape.pkgconfig");
        has(&l, "\\", "constant.character.escape.pkgconfig");
        // the next line is the value's, whatever it holds: pkgconf joins the two
        has(&l, "-DB", "variable.parameter.option.pkgconfig");
        has(&l, "Requires", "keyword.other.definition.pkgconfig");
        has(&l, "Libs:", "entity.name.package.pkgconfig");
        has(&l, "n", "entity.name.package.pkgconfig");
    }

    #[test]
    fn templates_placeholders() {
        let l = [
            "prefix=@CMAKE_INSTALL_PREFIX@",
            "@pc_data@",
            "Libs: -L${libdir} -lz@SUFFIX@",
            "Copyright: <a@b.c>, $@",
            "@NAME@_dir=x",
        ];
        has(&l, "@", "punctuation.definition.placeholder.pkgconfig");
        has(
            &l,
            "CMAKE_INSTALL_PREFIX",
            "variable.other.placeholder.pkgconfig",
        );
        has(&l, "pc_data", "variable.other.placeholder.pkgconfig");
        has(&l, "-lz", "variable.parameter.option.pkgconfig");
        has(&l, "SUFFIX", "variable.other.placeholder.pkgconfig");
        // an address and `$@` aren't placeholders
        has(&l, "<a@b.c>, $@", "string.unquoted.pkgconfig");
        has(&l, "NAME", "variable.other.placeholder.pkgconfig");
        has(&l, "_dir", "entity.name.variable.pkgconfig");
    }

    #[test]
    fn long_lines_lex_in_linear_time() {
        for unit in ["Requires: a >= 1, ", "x=${a}$$@b@\\#", "Libs: -la ", "${@"] {
            let line = unit.repeat(20_000);
            let start = std::time::Instant::now();
            marked(Lang::PkgConfig, &[&line]);
            assert!(start.elapsed().as_secs() < 2, "{unit:?}");
        }
    }
}
