//! Nix on a scanner of ada's own (X-103), from a floor probe: one loop over a line's bytes, a
//! byte's class one load from a table, a name read once and looked up once in one table (the
//! keywords, the constants, the builtins in Nix's global scope), the byte after it deciding a
//! lambda's parameter, and frames, four bits a level ([`Scan::frames`]), for what nests or runs
//! across lines: an attribute set or a function's formals, `let`'s bindings, a list, a
//! parenthesis, `inherit`'s names, `with`'s and `assert`'s head, `${…}`, a `"…"` or `''…''`
//! string, a path around an interpolation, a `/* */` comment. Kinds are bat's leaves; no theme
//! keys on a meta scope, so none are written.
//!
//! The readings that need an item's start: a binding's attribute path (its names bat's attribute
//! names, its dots plain, its `=` bat's bind) where a name in an attribute set or `let` is
//! followed by an attribute path and `=` on its line, and a function's formals (a name before
//! `,`, `?` or `}`).
//!
//! **Where the language and bat's grammar part, the language's reading** (each a departure):
//! - no `markup.italic` over an interpolation's text (bat's meta scope on `${…}`, which every
//!   built-in theme sets in italics: its blanks and plain bytes italic, a markup scope in code);
//! - `$${` is text, as Nix reads it (`$$` is two dollars; bat opens an interpolation at the
//!   second);
//! - a float is a number (`1.5`, `1.5e3`, `.5`; bat reads `1`, an operator and `5`, or
//!   `invalid`);
//! - a path runs on through an interpolation (`./foo/${name}.nix`, Nix 2.4's; bat ends it
//!   before the `${` and reads the rest as an operator and names);
//! - `TODO`, `FIXME`, `BUG` and `!!!` in a comment as words (bat's pattern finds `BUG` inside
//!   `DEBUG`), and on a block comment's first line too (bat's opener takes the line's text whole);
//! - `fetchTarball`, `fetchGit`, `fetchTree`, `fetchMercurial`, `fromTOML` and `placeholder`
//!   are builtins in Nix's global scope as `import` is (bat's list predates them);
//! - a name selected after `.` is a member, a builtin only under `builtins` (`builtins.readFile`;
//!   bat colors a member by its word: `lib.map` and `cfg.placeholder` a builtin,
//!   `builtins.readFile` a name);
//! - a name is Nix's, `-` and `'` in it (bat's `\b` finds `import` in `import-cargo`).
//!
//! **Languages in strings** (bat reads each as a string's text):
//! - **shell in a string** (X-147, X-152) that a derivation's phase or hook, NixOS's `script`
//!   and their kin hold (a binding's name, [`shell_name`]), that a shell writer's call takes
//!   (`writeShellScript`, `runCommand`, `writeShellApplication`'s set: [`W_SHELL`]), or that a
//!   `/* bash */` or `/* sh */` comment names: its text on bash's scanner ([`Scan::view`]), one
//!   script from the string's opener to its close, bash's state across its lines in the state's
//!   spare bytes ([`Ext`]). A `''…''` string is the script's at the value's level or deeper, a
//!   `"…"` string at the value's own level only (deeper it is an argument: a condition's, a
//!   separator); a writer's first argument, its name, is no script. An interpolation is Nix's
//!   island, a word to bash (`x`) where it holds no shell text; a `''…''` string inside one
//!   (`${lib.optionalString c ''…''}`) is the script's text spliced in, read on with the
//!   script's state, its own indentation stripped, but for a writer whose string isn't shell
//!   (`writeText`, `toFile`, `writePython3Bin`, `writeScript "a.py"`: [`held_writer`]), whose
//!   string is a file's.
//!   `''$`, `'''` and `''\x` are Nix's escapes, `\x` a `"…"` string's, cooked for bash. bash's
//!   kinds from [`SH_BASE`], no string scope over them (a here-document's body's reading,
//!   Ruby's).
//! - **a writer's file in its language** (X-153): the string a writer writes to a file
//!   (`writeText "a.json" ''…''`, `toFile`, `writeTextDir`, `writeScript "a.py"`: [`W_NAMED`],
//!   the language its name's extension says as ada names a file, [`ext_slot`]; `writePython3`,
//!   `writeLua`, `writeJS`, `writeHaskell`, `writeBabashka`, `writeGuile`, `writeRust`,
//!   `writeCBin`: the writer's own, [`W_FILE`]; a name without an extension: its first line's
//!   shebang's interpreter, [`shebang`]) on that language's scanner, through
//!   [`crate::lex_runs`] (ada's own copy of each), read as a script's text is
//!   ([`Scan::file_view`], its indentation stripped, an interpolation a placeholder, a `"…"`
//!   string's `\n` a line break, a `''…''` string in its interpolation spliced in but a
//!   writer's own: [`writer_string`]) where no shell string is open, or in a script's
//!   interpolation (X-156: `cp ${writeText "a.json" ''…''} $out`, the script's bash state kept
//!   above the frames meanwhile, [`Scan::stash`]). Its kinds in a window each ([`FILES`],
//!   [`fold`]), scoped by its language's base and its own; its state in bash's place by a map of
//!   the bytes its language uses ([`FileLang::keep`]), the string a string's text from the next
//!   line on where it doesn't fit. `.conf` names none (bat's nginx). A file named outside a
//!   writer's call too (X-157): `writeTextFile`'s set's `name` or `destination` ([`W_SET`]), an
//!   attribute path's string part before `.text` or a set's `text` binding
//!   (`environment.etc."a.toml".text`, `xdg.configFile."b.lua" = { text = … }`: [`path_file`]),
//!   a dotfile by its whole name (`home.file.".zshrc".text`).
//! - **a regex in the string after `builtins.match` or `builtins.split`** (X-148, X-152;
//!   `match` and `split` plain or selected from any set: `lib.match`, `with builtins; match`):
//!   POSIX's extended syntax, Nix's `std::regex::extended`, on [`crate::regex`]'s ERE; in a
//!   `"…"` string cooked through Nix's `\` escapes (`"\\."` is `\.`), in a `''…''` string its
//!   text as it is (`''\.''`), `${…}` an island. Its kinds from [`RE_BASE`] and
//!   [`RE_IND_BASE`], under the string's scope.

use crate::emit::{Rule, Rules, Run, Runs, Sink, Spans};
use crate::prim::{char_len, find_any, find_in};
use crate::regex;
use crate::words::{Words, text_len};
use crate::{Lang, Out, Span, State};

/// The leaves: a token's scopes.
static LEAVES: [&str; 39] = [
    "",
    "comment.line.number-sign.nix",
    "comment.line.number-sign.nix markup.bold.comment.nix",
    "comment.block.nix",
    "comment.block.nix markup.bold.comment.nix",
    "string.quoted.double.nix",
    "string.quoted.double.nix punctuation.definition.string.double.start.nix",
    "string.quoted.double.nix punctuation.definition.string.double.end.nix",
    "string.quoted.double.nix constant.character.escape.nix",
    "string.quoted.other.nix",
    "string.quoted.other.nix punctuation.definition.string.other.start.nix",
    "string.quoted.other.nix punctuation.definition.string.other.end.nix",
    "string.quoted.other.nix constant.character.escape.nix",
    "punctuation.section.embedded.begin.nix",
    "punctuation.section.embedded.end.nix",
    "string.unquoted.path.nix",
    "string.unquoted.spath.nix",
    "string.unquoted.url.nix",
    "variable.parameter.name.nix",
    "variable.parameter.function.4.nix",
    "variable.parameter.function.1.nix",
    "entity.other.attribute-name.multipart.nix",
    "entity.other.attribute-name.single.nix",
    "keyword.operator.bind.nix",
    "keyword.operator.nix",
    "punctuation.terminator.bind.nix",
    "punctuation.terminator.inherit.nix",
    "keyword.other.nix",
    "keyword.other.inherit.nix",
    "constant.language.nix",
    "support.function.nix",
    "constant.numeric.nix",
    "punctuation.definition.expression.nix",
    "punctuation.section.function.arguments.nix",
    "punctuation.definition.list.nix",
    "punctuation.definition.attrset-or-function.nix",
    "punctuation.definition.attrset.nix",
    "punctuation.definition.entity.function.nix",
    "punctuation.definition.function.nix",
];

const COMMENT: u16 = 1;
const COMMENT_REMARK: u16 = 2;
const BLOCK_COMMENT: u16 = 3;
const BLOCK_REMARK: u16 = 4;
const DQ: u16 = 5;
const DQ_BEGIN: u16 = 6;
const DQ_END: u16 = 7;
const DQ_ESCAPE: u16 = 8;
const IND: u16 = 9;
const IND_BEGIN: u16 = 10;
const IND_END: u16 = 11;
const IND_ESCAPE: u16 = 12;
const INTERP_BEGIN: u16 = 13;
const INTERP_END: u16 = 14;
const PATH: u16 = 15;
const SPATH: u16 = 16;
const URL: u16 = 17;
const NAME: u16 = 18;
const PARAM: u16 = 19;
const FORMAL: u16 = 20;
const ATTR: u16 = 21;
const ATTR_SINGLE: u16 = 22;
const BIND: u16 = 23;
const OPERATOR: u16 = 24;
const TERMINATOR: u16 = 25;
const INHERIT_END: u16 = 26;
const KEYWORD: u16 = 27;
const INHERIT: u16 = 28;
const CONSTANT: u16 = 29;
const SUPPORT: u16 = 30;
const NUMBER: u16 = 31;
const PARENS: u16 = 32;
const INHERIT_PARENS: u16 = 33;
const LIST: u16 = 34;
const BRACE: u16 = 35;
const ATTRSET: u16 = 36;
const FORMALS_END: u16 = 37;
const COLON: u16 = 38;

/// A regex in a `"…"` string (X-148): [`regex::KINDS`] kinds from here, under the string's
/// scope.
const RE_BASE: u16 = 64;
const _: () = assert!(LEAVES.len() <= RE_BASE as usize);
/// A regex in a `''…''` string (X-152): the same kinds from here, under its scope.
const RE_IND_BASE: u16 = RE_BASE + regex::KINDS as u16;
/// Shell in a string (X-147): bash's kinds, its twins too, from here.
const SH_BASE: u16 = 512;
const _: () = assert!(RE_IND_BASE as usize + regex::KINDS <= SH_BASE as usize);
/// A writer's file in its language (X-153): each language's kinds in a window of its own from
/// here ([`FILES`]), past bash's and its twins.
const FILE_BASE: u16 = SH_BASE + 2 * crate::bash::TWIN;

/// A language a writer's file is read in (X-153), by its slot (one more than its index in
/// [`FILES`]; 0 for none).
struct FileLang {
    lang: Lang,
    /// Its kinds' window among Nix's, from [`FILE_BASES`]: a kind past it (or one its fold
    /// drops, [`fold`]) is plain. bash's are in [`SH_BASE`]'s window, as a script's.
    size: u16,
    /// Its state's bytes kept in [`Ext::bash`], by position: `levels`' from 0, `withs`' from
    /// [`W_`], `arg`'s [`A_LO`] and [`A_HI`] ([`NO`] for none). Every other byte zero, or its
    /// reading stops ([`FL_LOST`]).
    keep: [u8; SH_HELD],
    /// Its depth kept too, up to 15, in bash's depth's place; else it is zero.
    deep: bool,
    /// The text an interpolation stands for, without file text in it.
    ph: u8,
}

/// [`FileLang::keep`]'s none: a scratch byte of [`image`]'s, always 0 there.
const NO: u8 = 31;
// [`FileLang::keep`]'s positions past `levels`' 14: `withs`' from `W_`, `arg`'s two bytes.
const W_: u8 = 14;
const A_LO: u8 = 28;
const A_HI: u8 = 29;

/// [`FileLang::keep`] from a list.
const fn keep(p: &[u8]) -> [u8; SH_HELD] {
    let mut k = [NO; SH_HELD];
    let mut i = 0;
    while i < p.len() {
        k[i] = p[i];
        i += 1;
    }
    k
}

const fn file(lang: Lang, size: u16, keep: [u8; SH_HELD], deep: bool, ph: u8) -> FileLang {
    FileLang {
        lang,
        size,
        keep,
        deep,
        ph,
    }
}

/// The languages a writer's file is read in, the smaller windows first (a theme's table by kind
/// grows to the largest met). Each one's state kept as the census of its own files (`x153/foot`)
/// uses it: SQL's levels, Python's frames, depth, open string and head (X-162), PHP's frames, its
/// layout's tag and its mode, C's and C++'s preprocessor withs.
static FILES: [FileLang; 21] = [
    file(
        Lang::Bash,
        0,
        keep(&[0, 1, 2, 3, 4, 5, 6, 7, 8]),
        true,
        b'x',
    ),
    file(Lang::Json, 32, keep(&[0]), false, b'0'),
    file(Lang::Toml, 256, keep(&[0, 1]), false, b'0'),
    file(Lang::Ini, 64, keep(&[]), false, b'x'),
    file(Lang::Properties, 64, keep(&[0]), false, b'x'),
    file(Lang::Diff, 64, keep(&[]), false, b'x'),
    file(Lang::Yaml, 512, keep(&[0, 1, 2, 3, 4, 5, 6]), false, b'x'),
    file(Lang::Sql, 128, keep(&[0, 1, 2, 3, 4, 5, 6, 7]), false, b'x'),
    file(
        Lang::Lua,
        128,
        keep(&[0, 1, 2, 3, 4, 5, W_ + 4, W_ + 5, A_LO]),
        true,
        b'x',
    ),
    file(
        Lang::Css,
        512,
        keep(&[0, 1, W_ + 2, W_ + 3, W_ + 4, W_ + 5]),
        true,
        b'x',
    ),
    file(
        Lang::EmacsLisp,
        512,
        keep(&[0, 1, 2, 3, 4, 5, 6, 7, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::Scheme,
        512,
        keep(&[0, 1, 2, 3, 4, 5, 6, 7, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::Clojure,
        512,
        keep(&[0, 1, 2, 3, 4, 5, 6, A_LO, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::C,
        512,
        keep(&[0, 1, W_ + 10, W_ + 11, W_ + 12, W_ + 13, A_LO, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::Cpp,
        512,
        keep(&[0, 1, W_ + 10, W_ + 11, W_ + 12, W_ + 13, A_LO, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::Haskell,
        1024,
        keep(&[0, 1, 2, 3, 4, 5, 6]),
        false,
        b'x',
    ),
    file(
        Lang::Rust,
        1024,
        keep(&[0, 1, 2, 3, 4, 5, 6, A_LO, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::JavaScript,
        1024,
        keep(&[0, 1, 2, 3, 4, 5, 6, A_LO, A_HI]),
        true,
        b'x',
    ),
    file(Lang::Xml, 4096, keep(&[0, 1, 2]), false, b'x'),
    file(
        Lang::Php,
        4096,
        keep(&[0, 1, 2, 3, 4, 5, W_ + 13, A_LO, A_HI]),
        true,
        b'x',
    ),
    file(
        Lang::Python,
        4096,
        // six frames, the open string's kind and the statement's head, the string's context
        keep(&[0, 1, 2, 3, 4, 5, A_LO, A_HI, W_ + 12]),
        true,
        b'x',
    ),
];
// Each language's slot, one more than its index in [`FILES`] (`file_slots_name_their_languages`).
const BASH_SLOT: u8 = 1;
const JSON_SLOT: u8 = 2;
const TOML_SLOT: u8 = 3;
const INI_SLOT: u8 = 4;
const PROPS_SLOT: u8 = 5;
const DIFF_SLOT: u8 = 6;
const YAML_SLOT: u8 = 7;
const SQL_SLOT: u8 = 8;
const LUA_SLOT: u8 = 9;
const CSS_SLOT: u8 = 10;
const EL_SLOT: u8 = 11;
const SCM_SLOT: u8 = 12;
const CLJ_SLOT: u8 = 13;
const C_SLOT: u8 = 14;
const CPP_SLOT: u8 = 15;
const HS_SLOT: u8 = 16;
const RS_SLOT: u8 = 17;
const JS_SLOT: u8 = 18;
const XML_SLOT: u8 = 19;
const PHP_SLOT: u8 = 20;
const PY_SLOT: u8 = 21;
/// A writer's file whose language its first line's shebang says (a script's or a file's
/// named without an extension: `writeScript "update"`): decided there ([`shebang`]), a
/// string's text where it names none.
const SHEBANG: u8 = FL_SLOT;
const _: () = assert!(FILES.len() < 32);

/// Each file language's window's first kind, by its index in [`FILES`].
static FILE_BASES: [u16; FILES.len()] = {
    let mut t = [0; FILES.len()];
    let mut at = FILE_BASE;
    let mut i = 0;
    while i < FILES.len() {
        t[i] = at;
        at += FILES[i].size;
        i += 1;
    }
    t
};

/// A file's kind `kind`, in its language of slot `slot`, as Nix's: in its window, or `None`
/// for plain. bash's are a script's.
#[inline(always)]
fn fold(slot: u8, kind: u16) -> Option<u16> {
    let i = usize::from(slot - 1);
    let f = &FILES[i];
    let k = match f.lang {
        Lang::Bash => return (kind < 2 * crate::bash::TWIN).then_some(SH_BASE + kind),
        Lang::Php => crate::php::fold(kind)?,
        _ => kind,
    };
    (k < f.size).then_some(FILE_BASES[i] + k)
}

/// The language and its own kind a file's kind `kind` (at least [`FILE_BASE`]) is, [`fold`]'s
/// inverse.
fn unfold(kind: u16) -> Option<(Lang, u16)> {
    let i = FILE_BASES.iter().rposition(|&b| b <= kind)?;
    let (f, k) = (&FILES[i], kind - FILE_BASES[i]);
    if k >= f.size {
        return None;
    }
    Some(match f.lang {
        Lang::Php => (f.lang, crate::php::unfold(k)),
        l => (l, k),
    })
}

/// The scopes under `source.nix` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= FILE_BASE {
        // a writer's file: its language's base and scopes, as in a file of its own
        return unfold(kind).map_or_else(Vec::new, |(lang, k)| {
            let mut out = vec![lang.base()];
            out.extend(lang.scopes(k));
            out
        });
    }
    if kind >= SH_BASE {
        return crate::bash::scopes(kind - SH_BASE);
    }
    if kind >= RE_BASE {
        let (string, base) = if kind >= RE_IND_BASE {
            ("string.quoted.other.nix", RE_IND_BASE)
        } else {
            ("string.quoted.double.nix", RE_BASE)
        };
        if usize::from(kind - base) >= regex::KINDS {
            return Vec::new();
        }
        let mut out = vec![string];
        out.extend(regex::scopes(kind - base));
        return out;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// The floor probe's build (X-103's `sol`, `ADA_PARSE_X103_SOL` set at build time): the loop,
/// the lookups and the frames without the readings that need an item's start (a binding's
/// attribute path, a function's formals) or `inherit`'s names.
const SOL: bool = option_env!("ADA_PARSE_X103_SOL").is_some();

// A byte's class ([`CLASS`]).
const C_OTHER: u8 = 0;
/// ` `, `\t`, `\r`, `\x0b`, `\x0c`.
const C_BLANK: u8 = 1;
/// `[A-Za-z_]`.
const C_NAME: u8 = 2;
const C_DIGIT: u8 = 3;
const C_DQUOTE: u8 = 4;
const C_SQUOTE: u8 = 5;
const C_HASH: u8 = 6;
const C_SLASH: u8 = 7;
const C_LBRACE: u8 = 8;
const C_RBRACE: u8 = 9;
const C_LBRACKET: u8 = 10;
const C_RBRACKET: u8 = 11;
const C_LPAREN: u8 = 12;
const C_RPAREN: u8 = 13;
const C_SEMI: u8 = 14;
const C_DOT: u8 = 15;
/// `= ! < > & | + - * ? : , @ ~`.
const C_OP: u8 = 16;
const C_DOLLAR: u8 = 17;
/// A byte past ASCII: never a token's in Nix's code.
const C_WIDE: u8 = 18;

static CLASS: [u8; 256] = {
    let mut t = [C_WIDE; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = match c as u8 {
            b' ' | b'\t' | b'\r' | 0x0b | 0x0c => C_BLANK,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => C_NAME,
            b'0'..=b'9' => C_DIGIT,
            b'"' => C_DQUOTE,
            b'\'' => C_SQUOTE,
            b'#' => C_HASH,
            b'/' => C_SLASH,
            b'{' => C_LBRACE,
            b'}' => C_RBRACE,
            b'[' => C_LBRACKET,
            b']' => C_RBRACKET,
            b'(' => C_LPAREN,
            b')' => C_RPAREN,
            b';' => C_SEMI,
            b'.' => C_DOT,
            b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'+' | b'-' | b'*' | b'?' | b':' | b','
            | b'@' | b'~' => C_OP,
            b'$' => C_DOLLAR,
            _ => C_OTHER,
        };
        c += 1;
    }
    t
};

/// A name's bytes after its first, Nix's `[a-zA-Z0-9_'-]`.
static NAME_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'_' || b == b'\'' || b == b'-';
        c += 1;
    }
    t
};

/// A path's bytes between its `/`s, Nix's `[a-zA-Z0-9._+-]`.
static PATH_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-');
        c += 1;
    }
    t
};

/// A URI's bytes after its scheme's `:`, Nix's `[a-zA-Z0-9%/?:@&=+$,_.!~*'-]`.
static URI_BYTE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'%' | b'/'
                    | b'?'
                    | b':'
                    | b'@'
                    | b'&'
                    | b'='
                    | b'+'
                    | b'$'
                    | b','
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'!'
                    | b'~'
                    | b'*'
                    | b'\''
            );
        c += 1;
    }
    t
};

#[inline(always)]
fn is_blank(c: u8) -> bool {
    CLASS[usize::from(c)] == C_BLANK
}

/// Where the blanks at `p` end.
#[inline(always)]
fn blanks(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_blank(b[p]) {
        p += 1;
    }
    p
}

/// Where the name that starts at `p` (a `[A-Za-z_]`) ends.
#[inline(always)]
fn name_end(b: &[u8], p: usize) -> usize {
    let mut e = p + 1;
    while e < b.len() && NAME_BYTE[usize::from(b[e])] {
        e += 1;
    }
    e
}

/// Where the run of path bytes at `p` ends.
#[inline(always)]
fn path_run(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && PATH_BYTE[usize::from(b[p])] {
        p += 1;
    }
    p
}

/// Whether `b[q..]` is `${`.
#[inline(always)]
fn interp_at(b: &[u8], q: usize) -> bool {
    b.get(q) == Some(&b'$') && b.get(q + 1) == Some(&b'{')
}

/// Where a path whose first segment's bytes end at `q` ends, Nix's `{PATH_CHAR}*(/{PATH_CHAR}+)+/?`:
/// `None` where no `/` and a path byte follow (or a `/` and `${`, a path's interpolation).
#[inline(always)]
fn path_end(b: &[u8], mut q: usize) -> Option<usize> {
    if b.get(q) != Some(&b'/') {
        return None;
    }
    match b.get(q + 1) {
        Some(&c) if PATH_BYTE[usize::from(c)] => {}
        _ if interp_at(b, q + 1) => return Some(q + 1),
        _ => return None,
    }
    while b.get(q) == Some(&b'/') && b.get(q + 1).is_some_and(|&c| PATH_BYTE[usize::from(c)]) {
        q = path_run(b, q + 1);
    }
    // a `/` at its end, or before an interpolation
    if b.get(q) == Some(&b'/') {
        q += 1;
    }
    Some(q)
}

// A word's flags, above its kind's byte ([`INFO`]).
/// `let`: [`F_LET`] pushed.
const W_LET: u32 = 1 << 8;
/// `in`: [`F_LET`] popped.
const W_IN: u32 = 1 << 9;
/// `with` and `assert`: [`F_WITH`] pushed, to their `;`.
const W_WITH: u32 = 1 << 10;
/// `inherit`: [`F_INHERIT`] pushed, to its `;`.
const W_INHERIT: u32 = 1 << 11;
/// A keyword: never an attribute's name.
const W_KEYWORD: u32 = 1 << 12;
/// What a word does past its kind.
const W_ACTS: u32 = W_LET | W_IN | W_WITH | W_INHERIT;
/// A name (no kind of its own) whose call takes a script: the strings after it at its level,
/// to the `;` that ends its item, are shell ([`Ext::sh_mark`]), but its first argument, its
/// name.
const W_SHELL: u32 = 1 << 13;
/// `match` and `split`: a `"…"` string after it is a regex.
const W_REGEX: u32 = 1 << 14;
/// A writer whose string isn't shell ([`text_writer`]).
const W_TEXT: u32 = 1 << 15;
/// A writer whose file's language its first argument's name says, by its extension (X-153:
/// `writeText "a.json"`, `toFile`, `writeScript "a.py"`).
const W_NAMED: u32 = 1 << 16;
/// A writer whose file's language it says: [`FILES`]' slot, from here (X-153: `writePython3`,
/// `writeLua`).
const FILE_SHIFT: u32 = 17;
const W_FILE: u32 = 31 << FILE_SHIFT;
/// A writer whose file its set names (X-157: `writeTextFile { name = "a.json"; text = ''…''; }`):
/// its `name` or `destination` the language, its `text` binding's value the file's string.
const W_SET: u32 = 1 << 22;
/// What sends a name to [`Scan::call`].
const W_CALLS: u32 = W_SHELL | W_REGEX | W_TEXT | W_NAMED | W_FILE | W_SET;

/// A kind as a word's value.
const fn k(kind: u16) -> u32 {
    kind as u32
}

/// A writer of a file in the language of slot `slot`, its string no shell.
const fn writes(slot: u8) -> u32 {
    W_TEXT | (slot as u32) << FILE_SHIFT
}

const WORD_LIST: &[(&[u8], u32)] = &[
    (b"let", k(KEYWORD) | W_LET | W_KEYWORD),
    (b"in", k(KEYWORD) | W_IN | W_KEYWORD),
    (b"with", k(KEYWORD) | W_WITH | W_KEYWORD),
    (b"assert", k(KEYWORD) | W_WITH | W_KEYWORD),
    (b"inherit", k(INHERIT) | W_INHERIT | W_KEYWORD),
    (b"rec", k(KEYWORD) | W_KEYWORD),
    (b"if", k(KEYWORD) | W_KEYWORD),
    (b"then", k(KEYWORD) | W_KEYWORD),
    (b"else", k(KEYWORD) | W_KEYWORD),
    (b"or", k(OPERATOR) | W_KEYWORD),
    (b"true", k(CONSTANT)),
    (b"false", k(CONSTANT)),
    (b"null", k(CONSTANT)),
    (b"builtins", k(CONSTANT)),
    // the builtins in Nix's global scope
    (b"scopedImport", k(SUPPORT)),
    (b"import", k(SUPPORT)),
    (b"isNull", k(SUPPORT)),
    (b"abort", k(SUPPORT)),
    (b"throw", k(SUPPORT)),
    (b"baseNameOf", k(SUPPORT)),
    (b"dirOf", k(SUPPORT)),
    (b"removeAttrs", k(SUPPORT)),
    (b"map", k(SUPPORT)),
    (b"toString", k(SUPPORT)),
    (b"derivationStrict", k(SUPPORT)),
    (b"derivation", k(SUPPORT)),
    (b"fetchTarball", k(SUPPORT)),
    (b"fetchGit", k(SUPPORT)),
    (b"fetchTree", k(SUPPORT)),
    (b"fetchMercurial", k(SUPPORT)),
    (b"fromTOML", k(SUPPORT)),
    (b"placeholder", k(SUPPORT)),
    // names: what their call's string is (X-147, X-148)
    (b"writeShellScript", W_SHELL),
    (b"writeShellScriptBin", W_SHELL),
    (b"writeShellApplication", W_SHELL),
    (b"runCommand", W_SHELL),
    (b"runCommandLocal", W_SHELL),
    (b"runCommandCC", W_SHELL),
    (b"runCommandNoCC", W_SHELL),
    (b"runCommandNoCCLocal", W_SHELL),
    (b"runCommandWith", W_SHELL),
    (b"writeBash", W_SHELL),
    (b"writeBashBin", W_SHELL),
    (b"writeDash", W_SHELL),
    (b"writeDashBin", W_SHELL),
    (b"match", W_REGEX),
    (b"split", W_REGEX),
    // writers whose string is a file's (X-152), in the language its name says (X-153)
    (b"writeText", W_TEXT | W_NAMED),
    (b"writeTextFile", W_TEXT | W_SET),
    (b"writeTextDir", W_TEXT | W_NAMED),
    (b"toFile", W_TEXT | W_NAMED),
    // a script, mostly shell with a shebang: a shell value's under a mark (X-152), else its
    // name's language
    (b"writeScript", W_NAMED),
    (b"writeScriptBin", W_NAMED),
    // nixpkgs' writers of another language's script (X-153)
    (b"writePython3", writes(PY_SLOT)),
    (b"writePython3Bin", writes(PY_SLOT)),
    (b"writePyPy3", writes(PY_SLOT)),
    (b"writePyPy3Bin", writes(PY_SLOT)),
    (b"writePyPy2", writes(PY_SLOT)),
    (b"writePyPy2Bin", writes(PY_SLOT)),
    (b"writeLua", writes(LUA_SLOT)),
    (b"writeLuaBin", writes(LUA_SLOT)),
    (b"writeJS", writes(JS_SLOT)),
    (b"writeJSBin", writes(JS_SLOT)),
    (b"writeHaskell", writes(HS_SLOT)),
    (b"writeHaskellBin", writes(HS_SLOT)),
    (b"writeBabashka", writes(CLJ_SLOT)),
    (b"writeBabashkaBin", writes(CLJ_SLOT)),
    (b"writeGuile", writes(SCM_SLOT)),
    (b"writeGuileBin", writes(SCM_SLOT)),
    (b"writeRust", writes(RS_SLOT)),
    (b"writeRustBin", writes(RS_SLOT)),
    (b"writeCBin", writes(C_SLOT)),
    // and of languages ada-parse doesn't read: no shell either
    (b"writeRuby", W_TEXT),
    (b"writeRubyBin", W_TEXT),
    (b"writePerl", W_TEXT),
    (b"writePerlBin", W_TEXT),
    (b"writeNu", W_TEXT),
    (b"writeNuBin", W_TEXT),
    (b"writeFish", W_TEXT),
    (b"writeFishBin", W_TEXT),
    (b"writeNim", W_TEXT),
    (b"writeNimBin", W_TEXT),
    (b"writeFSharp", W_TEXT),
];

const NW: usize = WORD_LIST.len();
static WORD_TEXT: [&[u8]; NW] = {
    let mut t: [&[u8]; NW] = [b""; NW];
    let mut i = 0;
    while i < NW {
        t[i] = WORD_LIST[i].0;
        i += 1;
    }
    t
};
static WORDS: Words<256, NW, { text_len(&WORD_TEXT) }> = Words::new(&WORD_TEXT);
/// What each word is by id: its kind in the low byte, its flags above.
static INFO: [u32; NW + 1] = {
    let mut t = [0u32; NW + 1];
    let mut i = 0;
    while i < NW {
        t[i + 1] = WORD_LIST[i].1;
        i += 1;
    }
    t
};

// The frames, four bits a level in [`Scan::frames`]; 0 is the file's top, or a level lost past
// 32 levels.
/// `{ … }`: an attribute set or a function's formals, not yet told apart.
const F_BRACE: u8 = 1;
/// `{ … }` known to be a function's formals (a `,` or `...` in it).
const F_FORMALS: u8 = 2;
/// `let`'s bindings, to `in`.
const F_LET: u8 = 3;
/// `[ … ]`.
const F_LIST: u8 = 4;
/// `( … )`.
const F_PARENS: u8 = 5;
/// `inherit`'s names, to `;`.
const F_INHERIT: u8 = 6;
/// `inherit ( … )`'s source.
const F_INHERIT_PARENS: u8 = 7;
/// `with`'s or `assert`'s head, to `;`.
const F_WITH: u8 = 8;
/// `${ … }`.
const F_INTERP: u8 = 9;
/// A `"…"` string.
const F_DQ: u8 = 10;
/// A `''…''` string.
const F_IND: u8 = 11;
/// A `/* … */` comment.
const F_COMMENT: u8 = 12;
/// A path under its interpolation: it goes on after the `}`.
const F_PATH: u8 = 13;
/// `...` at a line's end in formals: the next line's blanks before its `}` or `,` are the
/// ellipsis's, as bat's meta scope over them.
const F_ELLIPSIS: u8 = 14;
/// A string whose text is another language's: shell (X-147), or a writer's file (X-153,
/// [`Ext::fl_mark`]).
const F_SH: u8 = 15;

/// The levels a writer's file's mark and a regex's string are kept at (X-153): below it, so the
/// state's byte for either keeps [`RE_FILE`] above it.
const MARK_LEVELS: u8 = 127;

/// How many of bash's levels the state keeps under a shell string (the bottom ones dropped
/// past them, as bash's own drops its past 28).
const SH_HELD: usize = 9;

/// What the scan carries for the languages in strings (X-147, X-148, X-152, X-153), in the
/// state's spare bytes: `withs[2]` the shell mark, `withs[3]` the regex string's level or a
/// writer's file's mark with [`RE_FILE`] (never both: no regex opens under a file's mark, nor a
/// file's under a regex's string), `withs[4]` the string's indentation (its low seven bits) and
/// [`IND_DQ`], `withs[5..14]` bash's levels or a file's state; `arg` the regex's state or the
/// file's language (its low seven bits), [`EXT_LIVE`], bash's depth or a file's (bits 8 to
/// 11), [`EXT_CALL`], the hold above the mark (bits 13 to 15). All zero but under such a string
/// or a mark, so the state is as before elsewhere. A file open in a script's interpolation
/// (X-156) keeps the script's bash state above the frames instead ([`Scan::stash`]).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
struct Ext {
    /// One more than the level a shell value starts at: a `''…''` string opened at it or above
    /// is shell (a binding whose name holds a script: its value, `+`-joined strings and
    /// `lib.optionalString`'s too), or at it only where [`Ext::sh_call`] (a shell writer's
    /// script after its name and arguments, a `/* bash */` comment's string), a `"…"` string
    /// at it only ([`Scan::dq_shell_here`]); to the `;` at that level or a closer below it.
    /// bash's state is the value's: its strings one script. 0 for none.
    sh_mark: u8,
    /// One more than the level of a writer's call whose string is a file in a language ada
    /// reads (X-153: `writeText "a.json"`, `writePython3`), the string at that level after its
    /// name its text ([`Scan::file_here`]); or of a binding whose attribute path names the file
    /// (X-157: `"a.json".text = ''…''`, its value's string the file's); to the `;` at that level
    /// or a closer below it. Under [`FL_SET`] the level of a set naming the file, whose `text`
    /// binding moves it to its value (`writeTextFile { … }`, `"a.lua" = { … }`). The file's
    /// string opens where no shell string is, or in a script's interpolation (X-156). 0 for
    /// none. Beside [`Ext::sh_mark`]: [`Ext::marked`].
    fl_mark: u8,
    /// The mark is a call's: a string at its level only, not one in its arguments' sets
    /// (`runCommand "x" { a = writeText "a" ''…''; } ''…''`).
    sh_call: bool,
    /// One more than the level of a binding inside a binding's shell value whose name isn't a
    /// script's (`postPatch = let f = writeText "f" ''…''; in ''…''`), or of a writer's call
    /// whose string isn't shell under any mark (`${writeText "f" ''…''}`, [`text_writer`]): no
    /// string is shell in its value, to its `;` or a closer below it. Kept at most 7 levels
    /// above the mark. 0 for none.
    sh_hold: u8,
    /// The level of a `"…"` string that is a regex, 0 for none.
    re_at: u8,
    /// The regex's state ([`regex::embedded`]'s), while its string is open.
    re_st: u8,
    /// One more than the innermost open shell string's indentation (its low seven bits, up
    /// to 126), which Nix strips from each line of a `''…''` string (Nix takes the least of
    /// every line's with text; this the least of those read so far), 0 before it is known: a
    /// spliced string's while it is open, then the script's learned again from its next lines
    /// (one a line opens and closes keeps it); and [`IND_DQ`], the outermost shell string a
    /// `"…"` string (only that one can be: a `"…"` string in an interpolation is an argument).
    /// One byte, as the state keeps it.
    sh_ind: u8,
    /// bash's state under the open shell string: its levels held, bottom first, and how many.
    /// Under a writer's file, the file's language's state ([`FileLang::keep`]) and its depth.
    bash_depth: u8,
    bash: [u8; SH_HELD],
    /// The file's language, its slot in [`FILES`], and [`FL_OPEN`] once its string is: then
    /// that string's frame is [`F_SH`] at level `fl_mark - 1` (a script's below it only where
    /// it opened in the script's interpolation; above it, strings spliced into its text from
    /// its interpolations), its state in [`Ext::bash`]. [`FL_SET`] before it opens.
    fl_lang: u8,
}

impl Ext {
    /// One more than the innermost shell string's indentation, 0 before it is known.
    #[inline(always)]
    fn indent(&self) -> u8 {
        self.sh_ind & !IND_DQ
    }

    #[inline(always)]
    fn set_indent(&mut self, i: u8) {
        self.sh_ind = self.sh_ind & IND_DQ | i;
    }

    /// The outermost shell string is a `"…"` string.
    #[inline(always)]
    fn dq(&self) -> bool {
        self.sh_ind & IND_DQ != 0
    }

    /// A shell mark or a writer's file's is set: one load of the two bytes.
    #[inline(always)]
    fn marked(&self) -> bool {
        u16::from_ne_bytes([self.sh_mark, self.fl_mark]) != 0
    }

    /// A writer's file's string is open.
    #[inline(always)]
    fn file_open(&self) -> bool {
        self.fl_lang & FL_OPEN != 0
    }
}

/// `arg`'s bit for a state that holds an [`Ext`].
const EXT_LIVE: u16 = 0x80;
/// `arg`'s bit for [`Ext::sh_call`].
const EXT_CALL: u16 = 0x1000;
/// Where `arg` keeps [`Ext::sh_hold`], as levels above the mark (1 to 7).
const EXT_HOLD_SHIFT: u16 = 13;
/// [`Ext::sh_ind`]'s bit for a `"…"` shell string, above the indentation.
const IND_DQ: u8 = 0x80;
/// [`Ext::fl_lang`]'s bit for the file's string open, above its slot.
const FL_OPEN: u8 = 0x20;
/// [`Ext::fl_lang`]'s bit for a file whose state didn't fit on a line ([`Scan::file_view`]).
const FL_LOST: u8 = 0x40;
/// [`Ext::fl_lang`]'s bit, before the file's string opens (where [`FL_LOST`]'s is free), for a
/// mark on a set naming the file (X-157): its `text` binding's value the file's string.
const FL_SET: u8 = FL_LOST;
const FL_SLOT: u8 = 0x1f;
/// The levels the frames keep ([`Scan::frames`]): a script's bash state rides above the
/// outermost while a file in its interpolation is open ([`Scan::stash`]).
const KEPT: usize = 32;
/// `withs[3]`'s bit for a writer's file's mark in the regex's place ([`Ext`]).
const RE_FILE: u8 = 0x80;
/// The most [`Ext::sh_ind`]'s indentation keeps: one more than an indentation of 126.
const INDENT_MAX: usize = 127;
const _: () = assert!(SH_HELD < 16);

/// A line's shell text, as bash reads it, or a writer's file's, as its language does
/// ([`Scan::view`]).
#[derive(Default)]
struct ShellView {
    /// The script's text: the line's shell text from where the scan meets it, Nix's escapes
    /// cooked, a placeholder for an interpolation without shell text in it.
    cooked: Vec<u8>,
    /// The runs of `cooked` that are the line's bytes as they are: where each starts in
    /// `cooked`, in the line, and its length.
    segs: Vec<(u32, u32, u32)>,
    /// Open interpolations read from shell text: the level and where `cooked` stood.
    islands: Vec<(u32, u32)>,
    /// bash's spans on `cooked`; or the file's language's, as Nix's kinds ([`fold`]) less
    /// [`SH_BASE`], which each is written with.
    bash: Vec<Span>,
    /// A writer's file's reading ([`read_shell`]): the text an interpolation stands for, 0 for a
    /// script's.
    file: u8,
    /// A file's language's spans and runs as [`crate::lex_runs`] writes them.
    spans: Vec<Span>,
    runs: Vec<Run>,
    /// Where a file's `"…"` string's `\n` breaks its cooked text: each piece a line to its
    /// language.
    breaks: Vec<u32>,
    /// A file's line ([`FILE`]): how far its spans and the runs of the line's own text are
    /// written ([`Scan::shell_text`]; a script's in [`Scan::sh_i`] and `sh_j`).
    at: (usize, usize),
}

std::thread_local! {
    /// A script's line ([`Scan::view`]).
    static SHELL: std::cell::RefCell<ShellView> = std::cell::RefCell::new(ShellView::default());
    /// A writer's file's line ([`Scan::file_view`]), apart from a script's: a file in a script's
    /// interpolation leaves the script's spans on its line for the text after it (X-156).
    static FILE: std::cell::RefCell<ShellView> = std::cell::RefCell::new(ShellView::default());
}

/// Lex `b`, a line of Nix without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on::<_, false>(b, state, out, Spans::new());
}

/// [`line`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut k = line_on::<_, true>(b, state, out, Runs::new(rules, runs));
    k.end_line(b.len());
}

#[inline(always)]
fn line_on<K: Sink, const SHARED: bool>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
    let mut s = Scan::new(b, out, k);
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex_line::<SHARED>();
    s.k.close(s.out);
    *state = s.state();
    s.k
}

/// A block of Nix on one scan opened from `state` ([`crate::Carry`]).
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Spans::new());
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`]).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan::new(b"", &mut out, Runs::new(rules, runs));
    s.load(state);
    s.out.0.clear();
    s.k.open(s.out, 64);
    let rest = lines.pass(&mut s);
    *state = s.state();
    rest
}

impl crate::Carry for Scan<'_, '_, Spans> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry::<false>(unsafe { crate::line_for(b) });
        self.out.0
    }
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        // SAFETY: as above
        self.carry::<true>(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

/// What [`Scan`] keeps of a line's shell strings, not between lines: four bytes, reset with one
/// store a line.
#[derive(Clone, Copy, Default)]
#[repr(C, align(4))]
struct LineSh {
    /// The line's shell text has been read from where the scan met it ([`Scan::view`]).
    view: bool,
    /// Shell strings opened on the line and still open ([`Scan::opened_sh`],
    /// [`Scan::closed_sh`]).
    opened: u8,
    /// The indentation of the shell string open before the first of them.
    outer: u8,
    /// The line's file text has been read from where the scan met it ([`Scan::file_view`]).
    fview: bool,
}

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    /// What each open frame is ([`F_BRACE`]…), four bits a level, the innermost lowest; past 32
    /// levels the outer ones are lost. Above the outermost, zero but for a script's bash state
    /// while a file in its interpolation is open ([`Scan::stash`]), which rides there as the
    /// frames shift.
    frames: u128,
    /// How many frames are open, up to 255.
    depth: u8,
    /// At an item's start: a line's start, a `{`, a `;`, a `,`, `let`. Not kept between lines.
    start: bool,
    /// Where a binding's `=` is, while its attribute path is read past a part that isn't a
    /// name (a string, an interpolation), 0 for none; at [`Scan::bind_depth`]. Not kept
    /// between lines.
    bind_at: usize,
    bind_depth: u8,
    /// Where the last run of path bytes that started neither a path nor a URI ended: a start
    /// before it has the same answer (one search a run, not one a name in it). Not kept between
    /// lines.
    no_path: usize,
    /// The languages in strings' state ([`Ext`]), kept between lines.
    ext: Ext,
    /// Once the line's shell text has been read ([`LineSh::view`]), how far its spans waiting
    /// in [`SHELL`] are written (and the runs of the line's own text, `sh_j`).
    sh_i: usize,
    sh_j: usize,
    /// What the scan keeps of the line's shell strings ([`LineSh`]). Not kept between lines.
    ln: LineSh,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    #[inline(always)]
    fn new(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            frames: 0,
            depth: 0,
            start: true,
            bind_at: 0,
            bind_depth: 0,
            no_path: 0,
            ext: Ext::default(),
            sh_i: 0,
            sh_j: 0,
            ln: LineSh::default(),
            k,
            out,
        }
    }

    /// The scan's fields from `state`: the frames in its levels and the first two withs, and
    /// the languages in strings' where it holds them.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let mut w = [0u8; 16];
        w[..14].copy_from_slice(&state.levels);
        w[14] = state.withs[0];
        w[15] = state.withs[1];
        self.frames = u128::from_le_bytes(w);
        self.depth = state.depth;
        if state.arg != 0 {
            self.load_ext(state);
        }
    }

    #[cold]
    #[inline(never)]
    fn load_ext(&mut self, state: &State) {
        let mut bash = [0; SH_HELD];
        bash.copy_from_slice(&state.withs[5..5 + SH_HELD]);
        self.ext = Ext {
            sh_mark: state.withs[2],
            re_at: state.withs[3],
            re_st: (state.arg & 0x7f) as u8,
            sh_call: state.arg & EXT_CALL != 0,
            sh_hold: match (state.arg >> EXT_HOLD_SHIFT) as u8 {
                0 => 0,
                h => state.withs[2].saturating_add(h),
            },
            sh_ind: state.withs[4],
            bash_depth: (state.arg >> 8 & 15) as u8,
            bash,
            fl_mark: 0,
            fl_lang: 0,
        };
        // a writer's file's mark in the regex's place
        if state.withs[3] & RE_FILE != 0 {
            self.ext.fl_mark = state.withs[3] & !RE_FILE;
            self.ext.fl_lang = self.ext.re_st;
            self.ext.re_at = 0;
            self.ext.re_st = 0;
        }
    }

    /// The state the scan's last line left: every field as it is, so a block lexes as its
    /// lines do one at a time.
    #[inline(always)]
    fn state(&self) -> State {
        let w = self.frames.to_le_bytes();
        let mut s = State::default();
        s.levels.copy_from_slice(&w[..14]);
        s.withs[0] = w[14];
        s.withs[1] = w[15];
        s.depth = self.depth;
        let x = &self.ext;
        if x.sh_mark | x.re_at | x.sh_ind | x.bash_depth | x.fl_mark != 0 {
            self.save_ext(&mut s);
        }
        s
    }

    #[cold]
    #[inline(never)]
    fn save_ext(&self, s: &mut State) {
        let x = &self.ext;
        s.withs[2] = x.sh_mark;
        s.withs[3] = x.re_at;
        s.withs[4] = x.sh_ind;
        s.withs[5..5 + SH_HELD].copy_from_slice(&x.bash);
        s.arg = u16::from(x.re_st & 0x7f)
            | EXT_LIVE
            | u16::from(x.bash_depth) << 8
            | if x.sh_call { EXT_CALL } else { 0 }
            | u16::from(x.sh_hold.saturating_sub(x.sh_mark) & 7) << EXT_HOLD_SHIFT;
        if x.fl_mark != 0 {
            s.withs[3] = x.fl_mark | RE_FILE;
            s.arg = s.arg & !0x7f | u16::from(x.fl_lang & 0x7f);
        }
    }

    /// `b`, a block's next line, lexed on this scan, its sink closed; the scan's line an empty
    /// one again after.
    #[inline(always)]
    fn carry<const SHARED: bool>(&mut self, b: &'b [u8]) {
        self.k.start_line();
        self.b = b;
        self.lex_line::<SHARED>();
        self.k.close(self.out);
        self.b = b"";
    }

    /// A write: one call on the run sink ([`Sink::merge_call`]).
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge_call(&mut self.out, s, e, kind);
    }

    /// A write inlined, for the common tokens.
    #[inline(always)]
    fn put_hot(&mut self, s: usize, e: usize, kind: u16) {
        self.k.merge(&mut self.out, s, e, kind);
    }

    #[inline(always)]
    fn top(&self) -> u8 {
        (self.frames & 15) as u8
    }

    #[inline(always)]
    fn push(&mut self, f: u8) {
        self.frames = self.frames << 4 | u128::from(f);
        self.depth = self.depth.saturating_add(1);
    }

    #[inline(always)]
    fn pop(&mut self) {
        if self.depth > 0 {
            self.frames >>= 4;
            self.depth -= 1;
        }
    }

    #[inline(always)]
    fn set_top(&mut self, f: u8) {
        self.frames = self.frames & !15 | u128::from(f);
    }

    /// The line, from what the line before left: a string's or a comment's rest first; the
    /// loop out of line where `SHARED` ([`Scan::code_shared`]).
    #[inline(always)]
    fn lex_line<const SHARED: bool>(&mut self) {
        self.start = true;
        self.bind_at = 0;
        self.no_path = 0;
        self.ln = LineSh::default();
        let p = match self.top() {
            F_DQ if self.depth == self.ext.re_at => self.dq_regex(0),
            F_DQ => self.dq(0),
            F_IND if self.depth == self.ext.re_at => self.ind_regex(0),
            F_IND => self.ind(0),
            F_SH => self.sh(0),
            F_COMMENT => self.comment_rest(0),
            F_ELLIPSIS => self.ellipsis_rest(),
            _ => 0,
        };
        if SHARED {
            self.code_shared(p);
        } else {
            self.code(p);
        }
        // a path ends with its line
        if self.top() == F_PATH {
            self.pop();
        }
    }

    /// [`Scan::code`] out of line: on the run sink, one copy that the line API and a block's
    /// lines share; on the span sink, which ada doesn't hold, inlined.
    #[inline(never)]
    fn code_shared(&mut self, p: usize) {
        self.code(p);
    }

    /// The line's code from `p`. A string or a comment is read where it opens and runs to its
    /// close or the line's end, so the loop's frame is always code's.
    #[inline(always)]
    fn code(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            p = match CLASS[usize::from(c)] {
                C_BLANK => blanks(b, p + 1),
                C_NAME => self.name(p),
                C_DIGIT => self.number(p),
                C_DQUOTE => {
                    if !SOL && self.start {
                        self.lead(p);
                    }
                    if !SOL && self.ext.marked() {
                        // a writer's file's mark set: its string, or a shell string beside it
                        if self.ext.fl_mark != 0 {
                            if let Some(r) = self.dq_marked(p) {
                                p = r;
                                continue;
                            }
                        } else if self.dq_shell_here(p) {
                            p = self.sh_dq_open(p);
                            continue;
                        }
                    }
                    self.put_hot(p, p + 1, DQ_BEGIN);
                    self.push(F_DQ);
                    self.dq(p + 1)
                }
                C_SQUOTE if b.get(p + 1) == Some(&b'\'') => {
                    if !SOL && self.ext.marked() {
                        if self.ext.fl_mark != 0 {
                            if let Some(r) = self.sq_marked(p) {
                                p = r;
                                continue;
                            }
                        } else if self.shell_here() {
                            p = self.sh_open(p);
                            continue;
                        }
                    }
                    self.put_hot(p, p + 2, IND_BEGIN);
                    self.push(F_IND);
                    self.ind(p + 2)
                }
                C_HASH => self.line_comment(p),
                C_SLASH => self.slash(p),
                C_LBRACE => {
                    self.put_hot(p, p + 1, BRACE);
                    self.push(F_BRACE);
                    self.start = true;
                    p + 1
                }
                C_RBRACE => self.rbrace(p),
                C_LBRACKET => {
                    self.put_hot(p, p + 1, LIST);
                    self.push(F_LIST);
                    self.start = false;
                    p + 1
                }
                C_RBRACKET => {
                    self.unwind();
                    if self.top() == F_LIST {
                        self.put_hot(p, p + 1, LIST);
                        self.pop();
                        self.unmark();
                    }
                    p + 1
                }
                C_LPAREN => {
                    self.start = false;
                    if self.top() == F_INHERIT {
                        self.put(p, p + 1, INHERIT_PARENS);
                        self.push(F_INHERIT_PARENS);
                    } else {
                        self.put_hot(p, p + 1, PARENS);
                        self.push(F_PARENS);
                    }
                    p + 1
                }
                C_RPAREN => {
                    self.unwind();
                    match self.top() {
                        F_PARENS => {
                            self.put_hot(p, p + 1, PARENS);
                            self.pop();
                            self.unmark();
                        }
                        F_INHERIT_PARENS => {
                            self.put(p, p + 1, INHERIT_PARENS);
                            self.pop();
                        }
                        _ => {}
                    }
                    p + 1
                }
                C_SEMI => self.semi(p),
                C_DOT => self.dot(p),
                C_OP => self.op(p),
                C_DOLLAR if b.get(p + 1) == Some(&b'{') => {
                    if !SOL && self.start {
                        self.lead(p);
                    }
                    self.interp(p)
                }
                C_WIDE => p + char_len(c),
                _ => p + 1,
            };
        }
    }

    /// Before a bracket's closer: the frames of a head left open in it (`with`'s, `inherit`'s,
    /// `let`'s) closed with it.
    #[inline(always)]
    fn unwind(&mut self) {
        while matches!(self.top(), F_WITH | F_INHERIT | F_LET) {
            self.pop();
        }
    }

    /// After a closer's pop: a shell mark set above the level now on top dropped (its item
    /// closed with the bracket).
    #[inline(always)]
    fn unmark(&mut self) {
        if self.ext.marked() {
            self.unmark_on();
        }
    }

    #[cold]
    #[inline(never)]
    fn unmark_on(&mut self) {
        let d1 = self.depth.saturating_add(1);
        if self.ext.fl_mark > d1 {
            self.clear_file();
        }
        if self.ext.sh_mark > d1 {
            self.clear_mark();
        } else if self.ext.sh_hold > d1 {
            self.ext.sh_hold = 0;
        }
    }

    /// A binding's `;`, a mark set: a hold's or the mark's value ended there.
    #[cold]
    #[inline(never)]
    fn semi_mark(&mut self) {
        let d1 = self.depth.saturating_add(1);
        if self.ext.fl_mark == d1 {
            self.clear_file();
        }
        if self.ext.sh_hold == d1 {
            self.ext.sh_hold = 0;
        } else if self.ext.sh_mark == d1 {
            self.clear_mark();
        }
    }

    /// A binding of name `w` (its `=` read, a mark set or not): the shell value it starts, a
    /// hold on a binding's value it sits in, or a file's set's binding.
    #[inline(always)]
    fn bound(&mut self, w: &[u8]) {
        if self.shell_binding(w) {
            self.mark_shell(false);
        } else if self.ext.marked() {
            self.bound_marked(w);
        }
    }

    /// [`Scan::bound`] under a mark: in a set naming a file ([`FL_SET`], X-157), its `name` or
    /// `destination` the file's language (an extension ada reads in a file; one it doesn't drops
    /// the mark, none leaves it) and its `text` the binding whose value holds the file's string;
    /// under a shell mark, a hold on the value. `w` is the line's, its `=` after it.
    #[cold]
    #[inline(never)]
    fn bound_marked(&mut self, w: &[u8]) {
        let q = blanks(
            self.b,
            w.as_ptr() as usize - self.b.as_ptr() as usize + w.len(),
        );
        let x = &mut self.ext;
        if x.fl_lang & (FL_SET | FL_OPEN) == FL_SET && self.depth == x.fl_mark {
            match w {
                b"text" if self.depth < MARK_LEVELS => {
                    x.fl_mark = self.depth + 1;
                    x.fl_lang &= !FL_SET;
                }
                b"name" | b"destination" => match named(self.b, q + 1) {
                    0 => self.clear_file(),
                    SHEBANG => {}
                    s => x.fl_lang = FL_SET | s,
                },
                _ => {}
            }
        }
        if self.ext.sh_mark != 0 {
            self.hold(false);
        }
    }

    /// A binding inside a binding's shell value, its name no script's, or a writer's call whose
    /// string isn't shell (`text`, under any mark): a hold on its value.
    #[cold]
    #[inline(never)]
    fn hold(&mut self, text: bool) {
        let x = &mut self.ext;
        // none past the depth counter's saturation, where its level is the mark's: the state keeps
        // a hold as levels above the mark
        if (text || !x.sh_call)
            && x.sh_hold == 0
            && self.depth >= x.sh_mark
            && self.depth - x.sh_mark < 7
            && self.depth < u8::MAX
        {
            x.sh_hold = self.depth.saturating_add(1);
        }
    }

    /// The shell value ended: its mark and bash's state dropped (but a writer's file's, open
    /// in it).
    #[cold]
    #[inline(never)]
    fn clear_mark(&mut self) {
        self.ext.sh_mark = 0;
        self.ext.sh_call = false;
        self.ext.sh_hold = 0;
        if !self.ext.file_open() {
            self.ext.bash_depth = 0;
            self.ext.bash = [0; SH_HELD];
        }
    }

    /// A writer's file's mark ended, or its string closed: its language and state dropped.
    #[cold]
    #[inline(never)]
    fn clear_file(&mut self) {
        if self.ext.file_open() {
            self.ext.bash_depth = 0;
            self.ext.bash = [0; SH_HELD];
        }
        self.ext.fl_mark = 0;
        self.ext.fl_lang = 0;
    }

    /// A shell mark at the level on top, a call's or a binding's value's, unless one is set (a
    /// binding's takes a call's place: `writeShellApplication { text = ''…''; }`; a script's
    /// binding in a shell value doesn't, so `let script = …; in ''…''` keeps the value's);
    /// bash's state fresh.
    #[inline(always)]
    fn mark_shell(&mut self, call: bool) {
        if (self.ext.sh_mark == 0 || (self.ext.sh_call && !call)) && !self.ext.file_open() {
            self.ext.sh_mark = self.depth.saturating_add(1);
            self.ext.sh_call = call;
            self.ext.sh_hold = 0;
            self.ext.bash_depth = 0;
            self.ext.bash = [0; SH_HELD];
        }
    }

    /// A `"` at `p`, a writer's file's mark set: its string opened, or a shell string (none
    /// while a file's is open), or `None` for a string's.
    #[cold]
    #[inline(never)]
    fn dq_marked(&mut self, p: usize) -> Option<usize> {
        if self.file_here(p, true) {
            self.ext.fl_lang |= FL_OPEN;
            return Some(self.sh_dq_open(p));
        }
        (self.ext.sh_mark != 0 && !self.ext.file_open() && self.dq_shell_here(p))
            .then(|| self.sh_dq_open(p))
    }

    /// `''` at `p`, a writer's file's mark set: its string opened (in a script's interpolation
    /// too, [`Scan::file_in_script`]), a string spliced into an open file's text from its
    /// interpolation (X-156: `${optionalString c ''…''}`, but a writer's own string,
    /// [`writer_string`]), or a shell string (none while a file's is open), or `None` for a
    /// string's.
    #[cold]
    #[inline(never)]
    fn sq_marked(&mut self, p: usize) -> Option<usize> {
        if self.file_here(p, false) {
            if self.has_sh() {
                return Some(self.file_in_script(p));
            }
            self.ext.fl_lang |= FL_OPEN;
            return Some(self.sh_open(p));
        }
        if self.ext.file_open() {
            return (!writer_string(self.b, p)).then(|| self.sh_open(p));
        }
        (self.ext.sh_mark != 0 && self.shell_here()).then(|| self.sh_open(p))
    }

    /// Whether a string at `p` (a `"…"` one where `dq`) is a writer's file's (X-153): its
    /// mark's, at its call's level, not the writer's first argument (its name), nor a part of
    /// the attribute path a binding's mark is on ([`Scan::bind_at`]: `"a.json".text = …`), nor a
    /// `"…"` string after a name (a function's argument: `text = concatStringsSep "\n" …`). Where no
    /// shell string is open, none with bash's state from the value's strings before (the file's
    /// takes its place); in a script's interpolation (X-156) a `''…''` string, where the script's
    /// bash state fits above the frames ([`Scan::stash`]).
    #[inline(always)]
    fn file_here(&self, p: usize, dq: bool) -> bool {
        let x = &self.ext;
        if x.fl_mark == 0
            || x.fl_lang & (FL_OPEN | FL_SET) != 0
            || self.depth != x.fl_mark - 1
            || p < self.bind_at
            || writer_before(self.b, p)
            || (dq && after_name(self.b, p))
        {
            return false;
        }
        if self.has_sh() {
            !dq && usize::from(self.depth) + 3 + 2 * usize::from(x.bash_depth) <= KEPT
        } else {
            x.bash_depth == 0 && x.sh_ind == 0
        }
    }

    /// After a writer's name `w` ending at `e` whose file is in a language ada reads: its mark
    /// set, the language its first argument's name says (`writeText "a.json"`), the writer does
    /// (`writePython3`) or its set will (`writeTextFile { name = "a.json"; … }`, [`FL_SET`]).
    #[cold]
    #[inline(never)]
    fn file_call(&mut self, e: usize, w: u32) {
        let slot = if w & W_SET != 0 {
            FL_SET | SHEBANG
        } else if w & W_NAMED != 0 {
            named(self.b, e)
        } else {
            (w >> FILE_SHIFT) as u8 & FL_SLOT
        };
        self.file_mark(slot, text_writer(self.b, e, w));
    }

    /// A file's mark at the level on top, its language `slot` ([`FL_SET`] with it for a set's),
    /// its writer's string no script's where `text` ([`text_writer`]). None where a file's or a
    /// regex's mark is set (but a set's waiting for its `text`, which a writer's call in it
    /// takes the place of: `etc."a.xml" = { source = writeText "a.xml" ''…''; }`), past the
    /// levels its byte keeps, or for a script named as a shell's under a shell mark (X-152's
    /// reading: the value's script), but a text writer's in a script's interpolation (X-156:
    /// `${writeText "x" ''#!/bin/sh …''}`). A writer's file named without an extension in a
    /// set naming one takes the set's language (`"a.xml" = { source = writeText "a" ''…''; }`).
    fn file_mark(&mut self, mut slot: u8, text: bool) {
        let x = &self.ext;
        if x.fl_mark != 0 && slot & FL_SLOT == SHEBANG {
            slot = slot & FL_SET | x.fl_lang & FL_SLOT;
        }
        if slot & FL_SLOT == 0
            || (x.fl_mark != 0 && x.fl_lang & (FL_SET | FL_OPEN) != FL_SET)
            || x.re_at != 0
            || self.depth >= MARK_LEVELS
            || (matches!(slot & FL_SLOT, BASH_SLOT | SHEBANG)
                && self.ext.sh_mark != 0
                && !(text && self.has_sh()))
        {
            return;
        }
        self.ext.fl_mark = self.depth + 1;
        self.ext.fl_lang = slot;
        // its text read on its own, past a file before it on the line that turned out none
        self.ln.fview = false;
    }

    /// `''` at `p` opening a writer's file in a script's interpolation (X-156:
    /// `cp ${writeText "a.json" ''…''} $out`): the script's bash state kept above the frames
    /// ([`Scan::stash`]) while the file's takes its place.
    #[cold]
    #[inline(never)]
    fn file_in_script(&mut self, p: usize) -> usize {
        self.put(p, p + 2, IND_BEGIN);
        self.push(F_SH);
        self.stash();
        self.ext.fl_lang |= FL_OPEN;
        self.opened_sh();
        self.sh(p + 2)
    }

    /// The script's bash state, its levels held and how many, as nibbles above the outermost
    /// frame: how many plus one, two a level, how many plus one again, so the last is the first
    /// the frames shift out past the levels they keep ([`Scan::unstash`] finds none then);
    /// bash's state fresh for the file's.
    fn stash(&mut self) {
        let x = &mut self.ext;
        let n = x.bash_depth;
        let mut s = u128::from(n + 1);
        for (i, &l) in x.bash[..usize::from(n)].iter().enumerate() {
            s |= u128::from(l) << (4 + 8 * i);
        }
        s |= u128::from(n + 1) << (4 + 8 * u32::from(n));
        self.frames |= s << (4 * u32::from(self.depth));
        x.bash = [0; SH_HELD];
        x.bash_depth = 0;
    }

    /// The script's bash state back from above the frames ([`Scan::stash`]), the nibbles cleared,
    /// at a file's string's end; bash's state fresh where none is whole there.
    fn unstash(&mut self) {
        let d = u32::from(self.depth);
        if d as usize >= KEPT {
            return;
        }
        let s = self.frames >> (4 * d);
        self.frames &= (1u128 << (4 * d)) - 1;
        let n = (s & 15) as u32;
        if n == 0
            || n > SH_HELD as u32 + 1
            || d + 2 * n > KEPT as u32
            || (s >> (8 * n - 4)) & 15 != u128::from(n)
        {
            return;
        }
        let x = &mut self.ext;
        x.bash_depth = (n - 1) as u8;
        for (i, l) in x.bash[..(n - 1) as usize].iter_mut().enumerate() {
            *l = (s >> (4 + 8 * i)) as u8;
        }
    }

    /// The file's string closed (at its level, `fl_mark - 1`, after the pop): its mark and state
    /// dropped, the script's bash state back where it opened in its interpolation.
    #[cold]
    #[inline(never)]
    fn file_closed(&mut self) {
        self.clear_file();
        self.ln.fview = false;
        self.unstash();
    }

    /// A writer's file whose first line names no language ([`SHEBANG`]), or whose state didn't
    /// fit ([`FL_LOST`]): its string, at `lvl` from the top (a `"…"` one where `dq`), and those
    /// spliced into it above, strings', the line's text from `p` written as theirs by
    /// [`Scan::shell_text`] (one span of the string's kind over it), the next lines' as
    /// [`Scan::ind`] or [`Scan::dq`] read them; a script's bash state back from above the frames.
    fn not_a_file(&mut self, b: &[u8], p: usize, lvl: usize, dq: bool, v: &mut ShellView) {
        for i in 0..=lvl {
            if (self.frames >> (4 * i)) & 15 == u128::from(F_SH) {
                let f = if i == lvl && dq { F_DQ } else { F_IND };
                self.frames ^= u128::from(F_SH ^ f) << (4 * i);
            }
        }
        if dq {
            self.ext.sh_ind = 0;
        } else {
            self.ext.set_indent(0);
        }
        self.clear_file();
        self.unstash();
        let kind = if (self.frames & 15) as u8 == F_DQ {
            DQ
        } else {
            IND
        };
        let n = (b.len() - p.min(b.len())) as u32;
        v.segs.push((0, p as u32, n));
        v.bash.push(Span {
            start: 0,
            end: n,
            kind: kind.wrapping_sub(SH_BASE),
        });
    }

    /// Whether a `''` opened here is shell, a mark set: at the mark's level or above (a call's
    /// at it), or in a shell string's interpolation (spliced into its script); none under a
    /// hold.
    #[inline(always)]
    fn shell_here(&self) -> bool {
        let m = self.ext.sh_mark - 1;
        if self.ext.sh_hold != 0 {
            false
        } else if self.ext.sh_call {
            self.depth == m || self.has_sh()
        } else {
            self.depth >= m
        }
    }

    /// Whether a `"` at `p` opens a shell string, a mark set: at the value's own level (deeper
    /// a `"…"` string is an argument: a condition's, a separator, a writer's name), never in a
    /// shell string's interpolation, nor under a hold. At that level a writer's first
    /// argument, right after its name, is its name, and a comparison's operand (after `==` or
    /// `!=`) or a selected attribute's name (after `.`) no script either.
    #[cold]
    #[inline(never)]
    fn dq_shell_here(&self, p: usize) -> bool {
        let x = &self.ext;
        let b = self.b;
        let before = b[..p].trim_ascii_end();
        x.sh_hold == 0
            && self.depth == x.sh_mark - 1
            && !writer_before(b, p)
            && !before.ends_with(b"==")
            && !before.ends_with(b"!=")
            && !before.ends_with(b".")
            && !self.has_sh()
    }

    /// Whether a binding of name `w` here holds the shell value's script: a script's name
    /// ([`shell_name`]), or `text` in a shell writer's set (`writeShellApplication`'s).
    #[inline(always)]
    fn shell_binding(&self, w: &[u8]) -> bool {
        shell_name(w) || (self.ext.sh_call && self.depth == self.ext.sh_mark && w == b"text")
    }

    /// A name at `p`: a path or a URI it starts, a word, an attribute's name, a parameter, or a
    /// name.
    #[inline(always)]
    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let e = name_end(b, p);
        if let Some(&c) = b.get(e)
            && matches!(c, b'/' | b'.' | b'+' | b':')
            && e > self.no_path
            && let Some(r) = self.path_or_uri(p, e)
        {
            return r;
        }
        let w = WORDS.value(&b[p..e], &INFO);
        let top = self.top();
        if !SOL && self.start {
            self.start = false;
            if w & W_KEYWORD == 0
                && matches!(top, F_BRACE | F_FORMALS | F_LET)
                && let Some(r) = self.at_start(p, e, w, top)
            {
                return r;
            }
        }
        self.start = false;
        if !SOL && top == F_INHERIT && w & W_KEYWORD == 0 {
            self.put(p, e, ATTR_SINGLE);
            return e;
        }
        let kind = (w & 0xff) as u16;
        if kind != 0 {
            self.put_hot(p, e, kind);
            if w & W_ACTS != 0 {
                self.keyword(w);
            }
            return e;
        }
        let kind = match b.get(e) {
            Some(b':' | b'@') => PARAM,
            Some(b' ') if b.get(e + 1) == Some(&b':') => PARAM,
            _ => NAME,
        };
        self.put_hot(p, e, kind);
        if !SOL && w != 0 && kind == NAME {
            return self.call(e, w);
        }
        e
    }

    /// After a name `b[..e]` whose call's string is a language's ([`W_SHELL`], [`W_REGEX`]) or
    /// no shell under a mark ([`W_TEXT`]): a shell mark set, a regex's string opened, or a hold.
    #[cold]
    #[inline(never)]
    fn call(&mut self, e: usize, w: u32) -> usize {
        let b = self.b;
        if w & W_SHELL != 0 {
            self.mark_shell(true);
            return e;
        }
        if w & W_REGEX == 0 {
            if self.ext.sh_mark != 0 && held_writer(b, e, w) {
                // its string a file's, deeper than the value's level (at it, the value is the
                // file: `setupHook = writeText "x" ''…''` holds a script still)
                self.hold(true);
            }
            if w & (W_NAMED | W_FILE | W_SET) != 0 {
                self.file_call(e, w);
            }
            return e;
        }
        // none under a writer's file's mark, which takes the regex's place in the state, nor past
        // the levels its byte keeps
        if self.ext.fl_mark != 0 || self.depth >= MARK_LEVELS {
            return e;
        }
        let q = blanks(b, e);
        let ind = match b.get(q) {
            Some(b'"') => false,
            Some(b'\'') if b.get(q + 1) == Some(&b'\'') => true,
            _ => return e,
        };
        self.ext.re_st = 0;
        if ind {
            self.put(q, q + 2, IND_BEGIN);
            self.push(F_IND);
            self.ext.re_at = self.depth;
            return self.ind_regex(q + 2);
        }
        self.put(q, q + 1, DQ_BEGIN);
        self.push(F_DQ);
        self.ext.re_at = self.depth;
        self.dq_regex(q + 1)
    }

    /// A keyword's frame: `let`'s pushed, `in`'s popped, `with`'s, `assert`'s and `inherit`'s
    /// pushed to their `;`.
    #[inline(never)]
    fn keyword(&mut self, w: u32) {
        if w & W_LET != 0 {
            self.push(F_LET);
            self.start = true;
        } else if w & W_IN != 0 {
            if self.top() == F_LET {
                self.pop();
                self.unmark();
            }
        } else if w & W_WITH != 0 {
            self.push(F_WITH);
        } else {
            self.push(F_INHERIT);
        }
    }

    /// A name `b[p..e]` at an item's start in an attribute set, a function's formals or `let`:
    /// a binding's attribute path, or a formal; `None` for an expression's name.
    #[inline(always)]
    fn at_start(&mut self, p: usize, e: usize, w: u32, top: u8) -> Option<usize> {
        let b = self.b;
        let q = blanks(b, e);
        // a formal is a name; at a line's end, in formals or a brace's first
        let formal = w & 0xff == 0 && top != F_LET;
        match b.get(q) {
            Some(b'=') if b.get(q + 1) != Some(&b'=') => {
                self.put_hot(p, e, ATTR);
                self.put_hot(q, q + 1, BIND);
                if !SOL {
                    self.bound(&b[p..e]);
                }
                Some(q + 1)
            }
            Some(b'.') => self.binding(p, e),
            Some(b',' | b'?' | b'}') if formal => {
                self.put_hot(p, e, FORMAL);
                Some(e)
            }
            None if formal && (top == F_FORMALS || b[..p].trim_ascii_end().ends_with(b"{")) => {
                self.put_hot(p, e, FORMAL);
                Some(e)
            }
            _ => None,
        }
    }

    /// A name `b[p..e]` at an item's start with a `.` after it: a binding where the attribute
    /// path runs to an `=` (not `==`) on the line, its names attributes', its dots plain; a part
    /// that isn't a name (a string, an interpolation) read by the loop, the dots and names after
    /// it the path's still ([`Scan::bind_at`]).
    #[inline(never)]
    fn binding(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let (eq, other) = attr_path(b, e)?;
        let to = if other == 0 { eq } else { other };
        let mut s = p;
        let mut last = p..e;
        while s < to {
            let t = name_end(b, s);
            self.put(s, t, ATTR);
            last = s..t;
            s = blanks(b, t);
            if s < to {
                // the dot
                s = blanks(b, s + 1);
            }
        }
        if other == 0 {
            self.put_hot(eq, eq + 1, BIND);
            if !SOL {
                self.bound(&b[last]);
            }
            return Some(eq + 1);
        }
        self.bind_at = eq;
        self.bind_depth = self.depth;
        if !SOL {
            self.path_mark(p, eq);
        }
        Some(other)
    }

    /// A string or an interpolation at `p`, at an item's start in an attribute set or `let`:
    /// where it starts a binding's attribute path, its dots and names after it the path's
    /// ([`Scan::bind_at`]), and the file it names ([`Scan::path_mark`]); the loop reads it.
    #[inline(never)]
    fn lead(&mut self, p: usize) {
        self.start = false;
        if !matches!(self.top(), F_BRACE | F_LET) {
            return;
        }
        let b = self.b;
        let r = if b[p] == b'"' {
            dq_end(b, p + 1)
        } else {
            braces_end(b, p + 2)
        };
        if let Some((eq, _)) = r.and_then(|r| attr_path(b, r)) {
            self.bind_at = eq;
            self.bind_depth = self.depth;
            self.path_mark(p, eq);
        }
    }

    /// An attribute path `b[p..eq]` with a part that isn't a name: the file it names
    /// ([`path_file`]) marked on its value, or on its set ([`FL_SET`]), its own strings before
    /// its `=` no file's ([`Scan::file_here`]).
    #[cold]
    #[inline(never)]
    fn path_mark(&mut self, p: usize, eq: usize) {
        // a file's path ends in its name's string, or `text`
        let t = self.b[..eq].trim_ascii_end();
        if !(t.ends_with(b"\"") || t.ends_with(b"text")) {
            return;
        }
        match path_file(self.b, p, eq) {
            0 => {}
            slot => self.file_mark(slot, false),
        }
    }

    /// A name `b[p..e]` followed by a path's or a URI's bytes: the path or URI it starts, or
    /// `None`.
    #[inline(never)]
    fn path_or_uri(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let q = path_run(b, p);
        // a `'` in the name: no path's byte
        if q < e {
            return None;
        }
        if let Some(r) = path_end(b, q) {
            return Some(self.path(p, r));
        }
        self.no_path = q;
        // a URI: a scheme, `[a-zA-Z][a-zA-Z0-9+.-]*`, its `:` and a URI's byte
        if b.get(q) == Some(&b':')
            && b.get(q + 1).is_some_and(|&c| URI_BYTE[usize::from(c)])
            && !b[p..q].contains(&b'_')
        {
            let mut r = q + 1;
            while r < b.len() && URI_BYTE[usize::from(b[r])] {
                r += 1;
            }
            self.start = false;
            self.put(p, r, URL);
            return Some(r);
        }
        None
    }

    /// A path `b[p..r]`: written, and an interpolation that follows it opened, the path going
    /// on after its `}` ([`F_PATH`]).
    #[inline(never)]
    fn path(&mut self, p: usize, r: usize) -> usize {
        self.start = false;
        self.put(p, r, PATH);
        if interp_at(self.b, r) {
            self.push(F_PATH);
            return self.interp(r);
        }
        r
    }

    /// A path's rest from `p`, after an interpolation in it: its bytes and `/`s, another
    /// interpolation, or its end.
    #[inline(never)]
    fn path_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut r = p;
        while r < b.len() && (PATH_BYTE[usize::from(b[r])] || b[r] == b'/') {
            r += 1;
        }
        self.put(p, r, PATH);
        if interp_at(b, r) {
            return self.interp(r);
        }
        self.pop();
        r
    }

    /// A number at `p`: digits, a fraction and an exponent (Nix's float); a path it starts.
    #[inline(always)]
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.start = false;
        let mut e = p + 1;
        while e < n && b[e].is_ascii_digit() {
            e += 1;
        }
        if e < n
            && e > self.no_path
            && matches!(b[e], b'/' | b'.' | b'+' | b'-' | b'_' | b'a'..=b'z' | b'A'..=b'Z')
        {
            let q = path_run(b, p);
            if let Some(r) = path_end(b, q) {
                return self.path(p, r);
            }
            self.no_path = q;
            // Nix's float has a fraction, its exponent after it
            if b[e] == b'.' {
                e += 1;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                e = exponent(b, e);
            }
        }
        self.put_hot(p, e, NUMBER);
        e
    }

    /// A `.` at `p`: `...`, a path (`./`, `../`), a float (`.5`), an attribute path's dot, or a
    /// selection's operator.
    #[inline(always)]
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        self.start = false;
        match b.get(p + 1) {
            Some(b'.') if b.get(p + 2) == Some(&b'.') => return self.ellipsis(p),
            Some(b'.' | b'/') if p >= self.no_path => {
                let q = path_run(b, p);
                if let Some(r) = path_end(b, q) {
                    return self.path(p, r);
                }
                self.no_path = q;
            }
            Some(c) if c.is_ascii_digit() => {
                let mut e = p + 2;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                let e = exponent(b, e);
                self.put(p, e, NUMBER);
                return e;
            }
            _ => {}
        }
        if p < self.bind_at && self.depth == self.bind_depth {
            // an attribute path's dot, and its name
            let t = blanks(b, p + 1);
            if b.get(t).is_some_and(|&c| CLASS[usize::from(c)] == C_NAME) {
                let e = name_end(b, t);
                self.put(t, e, ATTR);
                return e;
            }
            return p + 1;
        }
        self.put_hot(p, p + 1, OPERATOR);
        // the member it selects: a builtin's name under `builtins`, else a name, whatever
        // word it is (`cfg.placeholder`, `lib.map`)
        let q = p + 1;
        if b.get(q).is_some_and(|&c| CLASS[usize::from(c)] == C_NAME) {
            let e = name_end(b, q);
            let kind = if after_builtins(b, p) { SUPPORT } else { NAME };
            self.put_hot(q, e, kind);
            // a call's name whose string is a language's (`match`, `split`, a shell writer's) or
            // a file's (the first byte and length first: a lookup only where one can be)
            if !SOL
                && matches!(
                    (b[q], e - q),
                    (b'm' | b's', 5) | (b'w', 7..) | (b'r', 9..) | (b't', 6)
                )
            {
                let w = WORDS.value(&b[q..e], &INFO);
                if w & W_CALLS != 0 {
                    return self.call(e, w);
                }
            }
            return e;
        }
        q
    }

    /// `...` at `p`: the formals' rest, with the blanks after it to a `}` or `,`, or to the
    /// line's end and on the next line's start ([`F_ELLIPSIS`]), as bat's meta scope over them.
    #[inline(never)]
    fn ellipsis(&mut self, p: usize) -> usize {
        let b = self.b;
        if self.top() == F_BRACE {
            self.set_top(F_FORMALS);
        }
        let q = blanks(b, p + 3);
        match b.get(q) {
            Some(b'}' | b',') => {
                self.put(p, q, OPERATOR);
                q
            }
            None if self.top() == F_FORMALS => {
                self.put(p, q, OPERATOR);
                self.push(F_ELLIPSIS);
                q
            }
            _ => {
                self.put(p, p + 3, OPERATOR);
                p + 3
            }
        }
    }

    /// A line after `...` at a line's end: its blanks before a `}` or `,` the ellipsis's.
    #[inline(never)]
    fn ellipsis_rest(&mut self) -> usize {
        let b = self.b;
        self.pop();
        let q = blanks(b, 0);
        match b.get(q) {
            Some(b'}' | b',') if q > 0 => {
                self.put(0, q, OPERATOR);
                q
            }
            None => {
                // a blank line: the ellipsis's still
                self.push(F_ELLIPSIS);
                q
            }
            _ => 0,
        }
    }

    /// A `/` at `p`: `//`, a comment, an absolute path, or the operator.
    #[inline(always)]
    fn slash(&mut self, p: usize) -> usize {
        let b = self.b;
        self.start = false;
        match b.get(p + 1) {
            Some(b'/') => {
                self.put_hot(p, p + 2, OPERATOR);
                p + 2
            }
            Some(b'*') => self.block_comment(p),
            _ => {
                if let Some(r) = path_end(b, p) {
                    return self.path(p, r);
                }
                self.put_hot(p, p + 1, OPERATOR);
                p + 1
            }
        }
    }

    /// An operator's byte at `p`.
    #[inline(always)]
    fn op(&mut self, p: usize) -> usize {
        let b = self.b;
        let next = b.get(p + 1).copied().unwrap_or(0);
        self.start = false;
        let (e, kind) = match b[p] {
            b'=' => match next {
                b'=' => (p + 2, OPERATOR),
                _ => {
                    self.bind_at = 0;
                    (p + 1, BIND)
                }
            },
            b'!' | b'<' | b'>' if next == b'=' => (p + 2, OPERATOR),
            b'<' => return self.spath(p),
            b'&' if next == b'&' => (p + 2, OPERATOR),
            b'|' if next == b'|' => (p + 2, OPERATOR),
            b'+' if next == b'+' => (p + 2, OPERATOR),
            b'-' if next == b'>' => (p + 2, OPERATOR),
            b'!' | b'>' | b'+' | b'-' | b'*' | b'?' => (p + 1, OPERATOR),
            b':' => (p + 1, COLON),
            b',' => {
                if self.top() == F_BRACE {
                    self.set_top(F_FORMALS);
                }
                self.start = !SOL;
                // a formal's default ended: a mark set in it (`runCommand ? pkgs.runCommand,`)
                if self.ext.marked() {
                    self.semi_mark();
                }
                (p + 1, OPERATOR)
            }
            b'~' => {
                if let Some(r) = path_end(b, p + 1) {
                    return self.path(p, r);
                }
                return p + 1;
            }
            _ => return p + 1,
        };
        self.put_hot(p, e, kind);
        e
    }

    /// `<` at `p`: a search path (`<nixpkgs/lib>`), or the operator.
    #[inline(never)]
    fn spath(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut q = path_run(b, p + 1);
        if q > p + 1 {
            while b.get(q) == Some(&b'/')
                && b.get(q + 1).is_some_and(|&c| PATH_BYTE[usize::from(c)])
            {
                q = path_run(b, q + 1);
            }
            if b.get(q) == Some(&b'>') {
                self.put(p, q + 1, SPATH);
                return q + 1;
            }
        }
        self.put_hot(p, p + 1, OPERATOR);
        p + 1
    }

    /// A `;` at `p`: `with`'s or `assert`'s (plain, as bat's), `inherit`'s, or a binding's.
    #[inline(always)]
    fn semi(&mut self, p: usize) -> usize {
        match self.top() {
            F_WITH => {
                self.pop();
                self.start = false;
            }
            F_INHERIT => {
                self.put(p, p + 1, INHERIT_END);
                self.pop();
                self.start = true;
            }
            _ => {
                self.put_hot(p, p + 1, TERMINATOR);
                self.start = true;
                // a binding's end: its shell mark's, or a writer's file's
                if self.ext.marked() {
                    self.semi_mark();
                }
            }
        }
        p + 1
    }

    /// A `}` at `p`: an attribute set's or formals' (the frames of a head left open in it closed
    /// with it), an interpolation's (whose string or path goes on), or none.
    #[inline(always)]
    fn rbrace(&mut self, p: usize) -> usize {
        self.start = false;
        self.unwind();
        match self.top() {
            F_INTERP => {
                self.put_hot(p, p + 1, INTERP_END);
                self.pop();
                // a mark or a hold set in it ends with it
                self.unmark();
                return match self.top() {
                    F_DQ if self.depth == self.ext.re_at => self.dq_regex(p + 1),
                    F_DQ => self.dq(p + 1),
                    F_IND if self.depth == self.ext.re_at => self.ind_regex(p + 1),
                    F_IND => self.ind(p + 1),
                    F_SH => self.sh(p + 1),
                    F_PATH => self.path_rest(p + 1),
                    _ => p + 1,
                };
            }
            F_BRACE => {
                let b = self.b;
                let q = blanks(b, p + 1);
                let kind = if matches!(b.get(q), Some(b':' | b'@')) {
                    FORMALS_END
                } else {
                    ATTRSET
                };
                self.put_hot(p, p + 1, kind);
                self.pop();
                self.unmark();
            }
            F_FORMALS => {
                self.put_hot(p, p + 1, FORMALS_END);
                self.pop();
                self.unmark();
            }
            _ => {}
        }
        p + 1
    }

    /// `${` at `p`: an interpolation opened.
    #[inline(always)]
    fn interp(&mut self, p: usize) -> usize {
        self.start = false;
        self.put_hot(p, p + 2, INTERP_BEGIN);
        self.push(F_INTERP);
        p + 2
    }

    /// A `"…"` string's text from `p`: to its close, the line's end, or an interpolation's
    /// opener. `\` escapes the character after it; `$$` is text, so `$${` is too.
    #[inline(always)]
    fn dq(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"\"\\$");
            if p >= n {
                if s < n {
                    self.put_hot(s, n, DQ);
                }
                return n;
            }
            match b[p] {
                b'"' => {
                    self.put_hot(s, p, DQ);
                    self.put_hot(p, p + 1, DQ_END);
                    self.pop();
                    return p + 1;
                }
                b'\\' => {
                    if p + 1 < n {
                        let e = p + 1 + char_len(b[p + 1]);
                        self.put_hot(s, p, DQ);
                        self.put(p, e.min(n), DQ_ESCAPE);
                        s = e.min(n);
                        p = s;
                    } else {
                        p += 1;
                    }
                }
                _ => match b.get(p + 1) {
                    Some(b'{') => {
                        self.put_hot(s, p, DQ);
                        return self.interp(p);
                    }
                    Some(b'$') => p += 2,
                    _ => p += 1,
                },
            }
        }
    }

    /// A `''…''` string's text from `p`: to its close, the line's end, or an interpolation's
    /// opener. `''$`, `'''` and `''\` with the character after it are escapes; `$$` is text.
    #[inline(always)]
    fn ind(&mut self, mut p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"'$");
            if p >= n {
                if s < n {
                    self.put_hot(s, n, IND);
                }
                return n;
            }
            if b[p] == b'\'' {
                if b.get(p + 1) != Some(&b'\'') {
                    p += 1;
                    continue;
                }
                let e = match b.get(p + 2) {
                    Some(b'$' | b'\'') => p + 3,
                    Some(b'\\') => (p + 3 + b.get(p + 3).map_or(0, |&c| char_len(c))).min(n),
                    _ => {
                        self.put_hot(s, p, IND);
                        self.put_hot(p, p + 2, IND_END);
                        self.pop();
                        return p + 2;
                    }
                };
                self.put_hot(s, p, IND);
                self.put(p, e, IND_ESCAPE);
                s = e;
                p = e;
                continue;
            }
            match b.get(p + 1) {
                Some(b'{') => {
                    self.put_hot(s, p, IND);
                    return self.interp(p);
                }
                Some(b'$') => p += 2,
                _ => p += 1,
            }
        }
    }

    /// Whether a shell string is open, in the frames kept.
    #[inline(always)]
    fn has_sh(&self) -> bool {
        sh_in(self.frames, self.depth)
    }

    /// `''` at `p` opening a shell string: the shell value's script going on (bash's state the
    /// value's), or text spliced into the open string's (a string in its interpolation).
    #[cold]
    #[inline(never)]
    fn sh_open(&mut self, p: usize) -> usize {
        self.put(p, p + 2, IND_BEGIN);
        self.push(F_SH);
        self.opened_sh();
        self.sh(p + 2)
    }

    /// `"` at `p` opening a shell string, the value's outermost.
    #[cold]
    #[inline(never)]
    fn sh_dq_open(&mut self, p: usize) -> usize {
        self.put(p, p + 1, DQ_BEGIN);
        self.push(F_SH);
        self.ext.sh_ind |= IND_DQ;
        self.opened_sh();
        self.sh_dq(p + 1)
    }

    /// A shell string opened: its indentation not yet known, the one before it kept for the
    /// line if it's the first opened on it (a string opened and closed on a line leaves the
    /// script's as it was).
    #[inline(always)]
    fn opened_sh(&mut self) {
        if self.ln.opened == 0 {
            self.ln.outer = self.ext.indent();
        }
        self.ln.opened = self.ln.opened.saturating_add(1);
        self.ext.set_indent(0);
    }

    /// A shell string closed, its frame popped: a writer's file's own ([`Scan::file_closed`]),
    /// then the outermost's end ([`Scan::end_string`]), or the indentation of the string it was
    /// spliced into (or the script a file opened in) back, as it was where the line opened it,
    /// or to be learned again from the next line.
    #[inline(always)]
    fn closed_sh(&mut self) {
        let back = if self.ln.opened > 0 {
            self.ln.opened -= 1;
            if self.ln.opened == 0 {
                self.ln.outer
            } else {
                0
            }
        } else {
            0
        };
        if self.ext.file_open() && self.depth < self.ext.fl_mark {
            self.file_closed();
        }
        if self.has_sh() {
            self.ext.set_indent(back);
        } else {
            self.end_string();
        }
    }

    /// The outermost shell string closed: its indentation and quote forgotten, the line's view
    /// to be read again where another opens on it (bash's state goes on, the value's).
    #[inline(always)]
    fn end_string(&mut self) {
        self.ext.sh_ind = 0;
        self.ln.view = false;
    }

    /// Whether the shell string on top is a `"…"` string: the outermost, open below no other.
    #[inline(always)]
    fn top_sh_dq(&self) -> bool {
        self.ext.dq() && !sh_in(self.frames >> 4, self.depth.saturating_sub(1))
    }

    /// Whether the line's text has been read from where the scan met it: a writer's file's
    /// (`file`, the text on top its own or spliced into it) or a script's.
    #[inline(always)]
    fn viewed(&self, file: bool) -> bool {
        if file { self.ln.fview } else { self.ln.view }
    }

    /// A shell string's text from `p`: to its close, the line's end, or an interpolation's
    /// opener, as [`Scan::ind`] reads it (a `"…"` string's as [`Scan::dq`] does), the text
    /// bash's spans ([`Scan::view`]), or a writer's file's its language's
    /// ([`Scan::file_view`]; the reading it is at the string's start kept to the line's end,
    /// where the file turns out none: [`Scan::not_a_file`]).
    #[cold]
    #[inline(never)]
    fn sh(&mut self, mut p: usize) -> usize {
        if self.top_sh_dq() {
            return self.sh_dq(p);
        }
        let file = self.ext.file_open();
        if !self.viewed(file) {
            self.view(p);
        }
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"'$");
            if p >= n {
                self.shell_text(file, s, n);
                return n;
            }
            if b[p] == b'\'' {
                if b.get(p + 1) != Some(&b'\'') {
                    p += 1;
                    continue;
                }
                let e = match b.get(p + 2) {
                    Some(b'$' | b'\'') => p + 3,
                    Some(b'\\') => (p + 3 + b.get(p + 3).map_or(0, |&c| char_len(c))).min(n),
                    _ => {
                        self.shell_text(file, s, p);
                        self.put(p, p + 2, IND_END);
                        self.pop();
                        self.closed_sh();
                        return p + 2;
                    }
                };
                self.shell_text(file, s, p);
                self.put(p, e, IND_ESCAPE);
                s = e;
                p = e;
                continue;
            }
            match b.get(p + 1) {
                Some(b'{') => {
                    self.shell_text(file, s, p);
                    return self.interp(p);
                }
                Some(b'$') => p += 2,
                _ => p += 1,
            }
        }
    }

    /// A `"…"` shell string's text from `p`, as [`Scan::dq`] reads it: `\` escapes the character
    /// after it, `$$` is text.
    #[cold]
    #[inline(never)]
    fn sh_dq(&mut self, mut p: usize) -> usize {
        let file = self.ext.file_open();
        if !self.viewed(file) {
            self.view(p);
        }
        let b = self.b;
        let n = b.len();
        let mut s = p;
        loop {
            p = find_any(b, p, *b"\"\\$");
            if p >= n {
                self.shell_text(file, s, n);
                return n;
            }
            match b[p] {
                b'"' => {
                    self.shell_text(file, s, p);
                    self.put(p, p + 1, DQ_END);
                    self.pop();
                    self.closed_sh();
                    return p + 1;
                }
                b'\\' => {
                    if p + 1 < n {
                        let e = (p + 1 + char_len(b[p + 1])).min(n);
                        self.shell_text(file, s, p);
                        self.put(p, e, DQ_ESCAPE);
                        s = e;
                        p = e;
                    } else {
                        p += 1;
                    }
                }
                _ => match b.get(p + 1) {
                    Some(b'{') => {
                        self.shell_text(file, s, p);
                        return self.interp(p);
                    }
                    Some(b'$') => p += 2,
                    _ => p += 1,
                },
            }
        }
    }

    /// The line's shell text `b[s..e]`: bash's spans there ([`Scan::view`]'s), or a writer's
    /// file's (`file`, [`Scan::file_view`]'s), laid on the line's bytes as they're written (each
    /// cut at the runs of the script's text that are the line's own, what covers none, a cooked
    /// escape or a placeholder, passed), clipped to it.
    #[inline(never)]
    fn shell_text(&mut self, file: bool, s: usize, e: usize) {
        if s >= e {
            return;
        }
        if file {
            FILE.with_borrow_mut(|v| v.at = self.lay(v, v.at, s, e));
        } else {
            let at = (self.sh_i, self.sh_j);
            (self.sh_i, self.sh_j) = SHELL.with_borrow(|v| self.lay(v, at, s, e));
        }
    }

    /// [`Scan::shell_text`]'s writes from `v`'s spans, from where they stand (`at`), to where.
    #[inline(always)]
    fn lay(&mut self, v: &ShellView, at: (usize, usize), s: usize, e: usize) -> (usize, usize) {
        let (sp, sg) = (&v.bash, &v.segs);
        let (mut i, mut j) = at;
        {
            while i < sp.len() && j < sg.len() {
                let (cs, ce) = (sp[i].start, sp[i].end);
                let (c0, r0, len) = sg[j];
                let c1 = c0 + len;
                if c1 <= cs {
                    j += 1;
                    continue;
                }
                if ce <= c0 {
                    i += 1;
                    continue;
                }
                let (ra, rz) = (
                    (r0 + cs.max(c0) - c0) as usize,
                    (r0 + ce.min(c1) - c0) as usize,
                );
                if ra >= e {
                    break;
                }
                if rz > s {
                    self.put(ra.max(s), rz.min(e), SH_BASE.wrapping_add(sp[i].kind));
                }
                if rz > e {
                    break;
                }
                if c1 >= ce {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
        (i, j)
    }

    /// Where the line's text from `p` is read from, a string's at level `lvl` from the top (a
    /// `"…"` one where `dq`): a line of a `''…''` string's own its indentation stripped, as Nix
    /// strips it (the least of its lines' with text so far, at most the line's own), a spliced
    /// string's its own; a `"…"` string's kept, as Nix keeps it; a line in code, every space.
    #[inline(always)]
    fn text_from(&mut self, p: usize, lvl: usize, dq: bool) -> usize {
        let b = self.b;
        if p != 0 || (dq && lvl == 0) {
            return p;
        }
        let sp = b.iter().take_while(|&&c| c == b' ').count();
        if self.top() != F_SH {
            return sp;
        }
        let i = self.ext.indent();
        if sp < b.len() && (i == 0 || sp + 1 < usize::from(i)) {
            self.ext.set_indent((sp + 1).min(INDENT_MAX) as u8);
        }
        sp.min(usize::from(self.ext.indent().saturating_sub(1)))
    }

    /// The line's shell text from `p`, where the scan first meets it (a shell string's text,
    /// its frame on top), read as bash reads it: to the line's end or the script's close, its
    /// frames followed as the scan follows them (an interpolation's code, a string in it, one
    /// spliced in), Nix's escapes cooked, an interpolation without shell text in it a word
    /// (`x`). bash's spans on it wait for the scan's text
    /// ([`Scan::shell_text`]); bash's state moves to the line's end. A reading of the frames
    /// apart from the scan's, so where the two part only the colors can be wrong: the scan
    /// writes bash's spans clipped to its own text. A writer's file's text, its own reading
    /// ([`Scan::file_view`]).
    #[cold]
    #[inline(never)]
    fn view(&mut self, p: usize) {
        if self.ext.file_open() {
            return self.file_view(p);
        }
        self.ln.view = true;
        self.sh_i = 0;
        self.sh_j = 0;
        let b = self.b;
        SHELL.with_borrow_mut(|v| {
            v.cooked.clear();
            v.segs.clear();
            v.islands.clear();
            v.bash.clear();
            let known = usize::from(self.depth).min(KEPT);
            // the levels from the outermost shell string open up to the top
            let Some(lvl) = (0..known)
                .rev()
                .find(|&i| (self.frames >> (4 * i)) & 15 == u128::from(F_SH))
            else {
                return;
            };
            let dq = self.ext.dq();
            let from = self.text_from(p, lvl, dq);
            read_shell(b, from, self.frames, lvl + 1, dq, v);
            let mut st = State {
                depth: self.ext.bash_depth,
                ..State::default()
            };
            for i in 0..usize::from(self.ext.bash_depth) {
                st.set_level(i, self.ext.bash[i]);
            }
            crate::bash::line_spans(&v.cooked, &mut st, &mut Out::new(&mut v.bash));
            // bash's levels, its bottom ones dropped past those kept
            let held = st.held();
            let from = held.saturating_sub(SH_HELD);
            for i in 0..SH_HELD {
                self.ext.bash[i] = if from + i < held {
                    st.level(from + i)
                } else {
                    0
                };
            }
            self.ext.bash_depth = (held - from) as u8;
        });
    }

    /// A writer's file's line from `p`, where the scan first meets its text (the file's string
    /// on top, or one spliced into it): its text from the file's string's level (`fl_mark - 1`)
    /// read as [`read_shell`] reads a script's (a `''…''` string in its interpolation spliced
    /// in, but a writer's own: [`writer_string`]), lexed by its language from its state in
    /// [`Ext::bash`] ([`unpack`]), its spans as Nix's kinds ([`fold`]) in [`FILE`], apart from a
    /// script's; the state kept again, or the language's reading lost where it doesn't fit
    /// ([`pack`]): a string's text from the next line on ([`FL_LOST`]). Where it turns out none,
    /// the line's text from `p` the string's ([`Scan::not_a_file`]).
    #[cold]
    #[inline(never)]
    fn file_view(&mut self, p: usize) {
        self.ln.fview = true;
        let b = self.b;
        FILE.with_borrow_mut(|v| {
            v.at = (0, 0);
            v.cooked.clear();
            v.segs.clear();
            v.islands.clear();
            v.bash.clear();
            // the file's string's level from the top, among those kept
            let (d, m) = (usize::from(self.depth), usize::from(self.ext.fl_mark));
            let Some(lvl) = d.checked_sub(m).filter(|&l| l < KEPT) else {
                return;
            };
            if (self.frames >> (4 * lvl)) & 15 != u128::from(F_SH) {
                return;
            }
            // a `"…"` string's quote is the outermost shell string's: the file's where no script
            // is open below it
            let below = self.frames.checked_shr(4 * (lvl as u32 + 1)).unwrap_or(0);
            let dq = self.ext.dq() && !sh_in(below, (m - 1) as u8);
            if self.ext.fl_lang & FL_LOST != 0 {
                return self.not_a_file(b, p, lvl, dq, v);
            }
            let from = self.text_from(p, lvl, dq);
            let mut slot = self.ext.fl_lang & FL_SLOT;
            let t = b[from.min(b.len())..].trim_ascii_start();
            if slot == SHEBANG
                || (slot == BASH_SLOT
                    && t.starts_with(b"#!nix-shell")
                    && self.ext.bash_depth == 0
                    && self.ext.bash == [0; SH_HELD])
            {
                // the language its shebang says, on its first line with text (or a `nix-shell`
                // shebang's `-i` on the next one, its state fresh still)
                if t.is_empty() {
                    return;
                }
                match shebang(t) {
                    0 if slot == SHEBANG => return self.not_a_file(b, p, lvl, dq, v),
                    0 => {}
                    s => {
                        slot = s;
                        self.ext.fl_lang = FL_OPEN | s;
                    }
                }
            }
            let Some(f) = FILES.get(usize::from(slot).wrapping_sub(1)) else {
                return;
            };
            v.file = f.ph;
            v.breaks.clear();
            read_shell(b, from, self.frames, lvl + 1, dq, v);
            let mut st = unpack(f, &self.ext.bash, self.ext.bash_depth);
            let ShellView {
                cooked,
                bash: out,
                spans,
                runs,
                breaks,
                ..
            } = v;
            // a line a piece, `"…"`'s `\n` between them
            let mut a = 0;
            for i in 0..=breaks.len() {
                let z = breaks.get(i).map_or(cooked.len(), |&z| z as usize);
                let piece = &cooked[a..z];
                let n = out.len();
                if f.lang == Lang::Bash {
                    crate::bash::line_spans(piece, &mut st, &mut Out::new(out));
                } else {
                    // the cooked text is the line's characters cut at ASCII bytes, and ASCII
                    let Ok(text) = std::str::from_utf8(piece) else {
                        return;
                    };
                    runs.clear();
                    crate::lex_runs(f.lang, text, &mut st, spans, &mut FileKinds, runs);
                    let mut at = 0;
                    for r in runs.iter() {
                        if r.rule.0 != Rule::NONE
                            && let Some(kind) = fold(slot, r.rule.0)
                        {
                            out.push(Span {
                                start: at,
                                end: r.end,
                                kind: kind.wrapping_sub(SH_BASE),
                            });
                        }
                        at = r.end;
                    }
                }
                for sp in &mut out[n..] {
                    sp.start += a as u32;
                    sp.end += a as u32;
                }
                a = z;
            }
            match pack(f, &st) {
                Some((bytes, depth)) => {
                    self.ext.bash = bytes;
                    self.ext.bash_depth = depth;
                }
                None => self.ext.fl_lang |= FL_LOST,
            }
        });
    }

    /// A regex's text in a `"…"` string from `p` (its frame on top, [`Ext::re_at`]), on
    /// [`crate::regex`]'s ERE: the string's end, its escapes and interpolations Nix's, the rest
    /// the regex's, its state kept across them and the string's lines.
    #[cold]
    #[inline(never)]
    fn dq_regex(&mut self, mut p: usize) -> usize {
        static YIELDS: [bool; 256] = regex::table(b"\"\\$");
        let b = self.b;
        let n = b.len();
        let host = regex::Embed {
            dialect: regex::Dialect::Ere,
            cooked: true,
            yields: &YIELDS,
            stop: |b, p, _| b[p] == b'"' || interp_in_text(b, p),
            arg: 0,
            escape: |b, p| (p + 1 + b.get(p + 1).map_or(0, |&c| char_len(c))).min(b.len()),
            top_only: false,
            continues: true,
            base: RE_BASE,
            text: DQ,
        };
        let mut re = self.ext.re_st;
        while p < n {
            match b[p] {
                b'"' => {
                    self.put(p, p + 1, DQ_END);
                    self.pop();
                    self.ext.re_at = 0;
                    self.ext.re_st = 0;
                    return p + 1;
                }
                b'\\' if b.get(p + 1) != Some(&b'\\') => {
                    let e = (p + 1 + b.get(p + 1).map_or(0, |&c| char_len(c))).min(n);
                    self.put(p, e, DQ_ESCAPE);
                    p = e;
                    continue;
                }
                b'$' if interp_in_text(b, p) => {
                    self.ext.re_st = re;
                    return self.interp(p);
                }
                _ => {}
            }
            self.k.close(self.out);
            let mut at = p;
            let r = regex::embedded(b, p, re, &host, &mut at, self.out);
            self.k.reopen(self.out, 16);
            match r {
                Some((e, st)) => {
                    re = st;
                    p = e.max(p + 1);
                }
                None => break,
            }
        }
        self.ext.re_st = re;
        n
    }

    /// A regex's text in a `''…''` string from `p` (its frame on top, [`Ext::re_at`]), on
    /// [`crate::regex`]'s ERE: the string's text the regex's as it is (a `\` the regex's), its
    /// end, its escapes (`''$`, `'''`, `''\x`) and interpolations Nix's, the regex's state kept
    /// across them and the string's lines.
    #[cold]
    #[inline(never)]
    fn ind_regex(&mut self, mut p: usize) -> usize {
        static YIELDS: [bool; 256] = regex::table(b"'$");
        let b = self.b;
        let n = b.len();
        let host = regex::Embed {
            dialect: regex::Dialect::Ere,
            cooked: false,
            yields: &YIELDS,
            stop: |b, p, _| b[p..].starts_with(b"''") || interp_in_text(b, p),
            arg: 0,
            escape: |_, p| p,
            top_only: false,
            continues: false,
            base: RE_IND_BASE,
            text: IND,
        };
        let mut re = self.ext.re_st;
        while p < n {
            if b[p..].starts_with(b"''") {
                let e = match b.get(p + 2) {
                    Some(b'$' | b'\'') => p + 3,
                    Some(b'\\') => (p + 3 + b.get(p + 3).map_or(0, |&c| char_len(c))).min(n),
                    _ => {
                        self.put(p, p + 2, IND_END);
                        self.pop();
                        self.ext.re_at = 0;
                        self.ext.re_st = 0;
                        return p + 2;
                    }
                };
                self.put(p, e, IND_ESCAPE);
                p = e;
                continue;
            }
            if interp_in_text(b, p) {
                self.ext.re_st = re;
                return self.interp(p);
            }
            self.k.close(self.out);
            let mut at = p;
            let r = regex::embedded(b, p, re, &host, &mut at, self.out);
            self.k.reopen(self.out, 16);
            match r {
                Some((e, st)) => {
                    re = st;
                    p = e.max(p + 1);
                }
                None => break,
            }
        }
        self.ext.re_st = re;
        n
    }

    /// A `#` comment at `p`.
    #[inline(always)]
    fn line_comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.remarks(p, n, COMMENT, COMMENT_REMARK);
        n
    }

    /// A comment's text `b[s..e]` as `kind`, its remarks (`TODO`, `FIXME`, `BUG`, `!!!`) as
    /// `remark`.
    #[inline(always)]
    fn remarks(&mut self, mut s: usize, e: usize, kind: u16, remark: u16) {
        let b = self.b;
        let mut q = s;
        loop {
            q = find_in(b, q, e, *b"TFB!");
            if q >= e {
                break;
            }
            let r = remark_end(b, q, e);
            if r > q {
                self.put_hot(s, q, kind);
                self.put(q, r, remark);
                s = r;
                q = r;
            } else {
                q += 1;
            }
        }
        if s < e {
            self.put_hot(s, e, kind);
        }
    }

    /// `/*` at `p`.
    #[inline(never)]
    fn block_comment(&mut self, p: usize) -> usize {
        self.push(F_COMMENT);
        self.put(p, p + 2, BLOCK_COMMENT);
        let r = self.comment_rest(p + 2);
        let b = self.b;
        if !SOL
            && r >= p + 4
            && b[r - 2..r] == *b"*/"
            && (b[blanks(b, r)..].starts_with(b"''") || b.get(blanks(b, r)) == Some(&b'"'))
        {
            self.annotation(b[p + 2..r - 2].trim_ascii());
        }
        r
    }

    /// A comment `/* word */` before a string, naming its language: `/* bash */ ''…''` shell;
    /// another's ends a comment's or a writer's mark at its level (`[ /* bash */ ''…'' /*
    /// ruby */ ''…'' ]`).
    #[cold]
    #[inline(never)]
    fn annotation(&mut self, word: &[u8]) {
        if matches!(word, b"bash" | b"sh") {
            self.mark_shell(true);
        } else if self.ext.sh_call
            && self.ext.sh_mark == self.depth.saturating_add(1)
            && !word.is_empty()
            && word.iter().all(u8::is_ascii_alphanumeric)
        {
            self.clear_mark();
        }
    }

    /// An open `/* */` comment from `p`: to its `*/`, or the line's end.
    #[inline(never)]
    fn comment_rest(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        loop {
            q = find_any(b, q, *b"*");
            if q + 1 >= n {
                self.remarks(p, n, BLOCK_COMMENT, BLOCK_REMARK);
                return n;
            }
            if b[q + 1] == b'/' {
                self.remarks(p, q + 2, BLOCK_COMMENT, BLOCK_REMARK);
                self.pop();
                return q + 2;
            }
            q += 1;
        }
    }
}

/// An attribute path's rest from `r`, past its first part: `(.` part`)*` and an `=` (not `==`)
/// on the line, a part a name, a `"…"` string or `${…}`. Where its `=` is, and where its first
/// part that isn't a name starts (its `.`), 0 for none; `None` for no binding's.
fn attr_path(b: &[u8], mut r: usize) -> Option<(usize, usize)> {
    let n = b.len();
    let mut other = 0;
    loop {
        let q = blanks(b, r);
        match b.get(q) {
            Some(b'.') => {
                let t = blanks(b, q + 1);
                match b.get(t) {
                    Some(&c) if CLASS[usize::from(c)] == C_NAME => r = name_end(b, t),
                    Some(b'"') => {
                        if other == 0 {
                            other = q;
                        }
                        r = dq_end(b, t + 1)?;
                    }
                    Some(b'$') if b.get(t + 1) == Some(&b'{') => {
                        if other == 0 {
                            other = q;
                        }
                        r = braces_end(b, t + 2)?;
                    }
                    _ => return None,
                }
            }
            Some(b'=') if b.get(q + 1) != Some(&b'=') => return Some((q, other)),
            _ => return None,
        }
        if r >= n {
            return None;
        }
    }
}

/// Whether a binding of this name holds a script (X-147): a derivation's phase or hook
/// (`installPhase`, `postPatch`, `preConfigure`, `shellHook`), NixOS's `script` (and
/// `preStart`, `postStop`), and their kin (`buildCommand`, `unpackCmd`, `setSourceRoot`,
/// `extraInstallCommands`, devenv's `enterShell` and `enterTest`). tree-sitter-nix's injections
/// read `*Phase`, `pre*`, `post*` and `script` the same; not `preConfig` and `postConfig`
/// (nixvim's Lua, home-manager's Firefox `user.js`), nor a `pre*` or `post*` message (X-152).
#[inline(always)]
fn shell_name(w: &[u8]) -> bool {
    let n = w.len();
    if n < 5 {
        return false;
    }
    // the last byte first: most names end in neither
    match w[n - 1] {
        b'e' if w.ends_with(b"Phase") && n > 5 => return true,
        b'k' if w.ends_with(b"Hook") => return true,
        _ => {}
    }
    match w[0] {
        b'p' => {
            ((w[1] == b'r' && w[2] == b'e' && w[3].is_ascii_uppercase())
                || (w[1] == b'o' && w[2] == b's' && w[3] == b't' && w[4].is_ascii_uppercase()))
                // a plugin's or a profile's code (nixvim's Lua, Firefox's `user.js`), a message
                && !matches!(w, b"preConfig" | b"postConfig")
                && !w.ends_with(b"Message")
        }
        b's' => matches!(w, b"script" | b"setSourceRoot"),
        b'b' => w == b"buildCommand",
        b'u' => w == b"unpackCmd",
        b'e' => matches!(w, b"extraInstallCommands" | b"enterShell" | b"enterTest"),
        _ => false,
    }
}

/// Whether a writer's call, its name `w` ([`INFO`]'s) ending at `e`, takes a string that isn't
/// shell (X-152): one of [`W_TEXT`]'s, writing a file's text or another language's script
/// (`writeText`, `writeTextFile`, `toFile`, `writePython3Bin`), its first argument after it, a
/// name (`"…"`, `(…)`) or a set, but a file named as a shell script's (`"hook.sh"`).
/// `writeScript` and `writeScriptBin` aren't [`W_TEXT`]'s: their scripts are mostly shell, a
/// shebang first. Its value is a store path, so a string in it isn't the script's even in a
/// shell string's interpolation.
#[inline(always)]
fn text_writer(b: &[u8], e: usize, w: u32) -> bool {
    if w & W_TEXT == 0 {
        return false;
    }
    let q = blanks(b, e);
    match b.get(q) {
        Some(b'"') => dq_end(b, q + 1).is_none_or(|r| {
            let name = &b[q + 1..r - 1];
            !(name.ends_with(b".sh") || name.ends_with(b".bash"))
        }),
        Some(b'{' | b'(') => true,
        _ => false,
    }
}

/// Whether a writer's call, its name `w` ending at `e`, takes a string that isn't shell: a text
/// writer's ([`text_writer`]), or a script's its name says is another language's (X-156:
/// `writeScript "a.py"`), a file's then in a shell string's interpolation.
fn held_writer(b: &[u8], e: usize, w: u32) -> bool {
    text_writer(b, e, w) || (w & W_NAMED != 0 && !matches!(named(b, e), 0 | BASH_SLOT | SHEBANG))
}

/// Rules whose rule for a kind is the kind, for a writer's file's language's runs on
/// [`crate::lex_runs`]: its runs are its spans ([`Scan::file_view`]). bash's own (`bash::Kinds`)
/// stays its scan's alone.
struct FileKinds;

impl Rules for FileKinds {
    fn table(&self) -> &[Rule] {
        &[]
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        Rule(kind)
    }
}

/// A state's bytes as [`FileLang::keep`] names them: `levels`, `withs`, `arg`, and [`NO`]'s
/// scratch byte (0).
#[inline(always)]
fn image(s: &State) -> [u8; 32] {
    let mut img = [0; 32];
    img[..14].copy_from_slice(&s.levels);
    img[14..28].copy_from_slice(&s.withs);
    img[28..30].copy_from_slice(&s.arg.to_le_bytes());
    img
}

/// A writer's file's state in language `f` from the bytes [`FileLang::keep`] names and its
/// depth, the rest zero.
fn unpack(f: &FileLang, bytes: &[u8; SH_HELD], depth: u8) -> State {
    let mut img = [0u8; 32];
    for (&p, &v) in f.keep.iter().zip(bytes) {
        img[usize::from(p & 31)] = v;
    }
    let mut s = State::default();
    s.levels.copy_from_slice(&img[..14]);
    s.withs.copy_from_slice(&img[14..28]);
    s.arg = u16::from_le_bytes([img[28], img[29]]);
    s.depth = if f.deep { depth } else { 0 };
    s
}

/// A writer's file's state `s` in language `f`, as [`Ext::bash`] and its depth keep it: the
/// bytes [`FileLang::keep`] names; `None` where another is set or its depth doesn't fit.
fn pack(f: &FileLang, s: &State) -> Option<([u8; SH_HELD], u8)> {
    let mut img = image(s);
    let mut out = [0; SH_HELD];
    for (o, &p) in out.iter_mut().zip(&f.keep) {
        *o = std::mem::take(&mut img[usize::from(p & 31)]);
    }
    let depth = if f.deep { s.depth } else { 0 };
    (img == [0; 32] && depth == s.depth && depth < 16).then_some((out, depth))
}

/// The slot in [`FILES`] of the language a shebang `t` (a line's text from its `#!`) names by
/// its interpreter: the word after `#!` (after `env` and its flags), its last path segment or
/// an interpolation's last name (`${pkgs.bash}/bin/bash`, `${runtimeShell}`,
/// `${python3.interpreter}`), `nix-shell`'s `-i` (bash where none is on the line: its next
/// `#!nix-shell` line may name another); 0 for none.
fn shebang(t: &[u8]) -> u8 {
    let Some(r) = t.strip_prefix(b"#!") else {
        return 0;
    };
    let mut words = r
        // a `"…"` string's first line ends at its `\n`
        .split(|&c| matches!(c, b' ' | b'\t' | b'\\'))
        .filter(|w| !w.is_empty());
    let Some(mut w) = words.next() else {
        return 0;
    };
    let last = |w: &[u8]| -> usize { w.iter().rposition(|&c| c == b'/').map_or(0, |i| i + 1) };
    if &w[last(w)..] == b"env" {
        let Some(x) = words.find(|w| !w.starts_with(b"-")) else {
            return 0;
        };
        w = x;
    }
    if w.ends_with(b"nix-shell") {
        return match words.skip_while(|&w| w != b"-i").nth(1) {
            Some(i) => interpreter(i),
            None => BASH_SLOT,
        };
    }
    interpreter(w)
}

/// The slot of the language an interpreter's path or name `w` is (a shebang's).
fn interpreter(w: &[u8]) -> u8 {
    let python = w.windows(6).any(|x| x == b"python");
    // the path's last segment, or its interpolation's last name
    let w = w.trim_ascii_end();
    let w = w.strip_suffix(b"}").unwrap_or(w);
    let i = w
        .iter()
        .rposition(|&c| matches!(c, b'/' | b'.' | b'{'))
        .map_or(0, |i| i + 1);
    let name = &w[i..];
    let shell = matches!(
        name,
        b"bash"
            | b"sh"
            | b"ash"
            | b"dash"
            | b"ksh"
            | b"mksh"
            | b"runtimeShell"
            | b"shell"
            | b"bashInteractive"
    );
    if shell {
        BASH_SLOT
    } else if python || name.starts_with(b"pypy") {
        PY_SLOT
    } else if name.starts_with(b"lua") {
        LUA_SLOT
    } else if matches!(name, b"node" | b"nodejs") {
        JS_SLOT
    } else if name == b"php" {
        PHP_SLOT
    } else {
        0
    }
}

/// The slot in [`FILES`] of the language a writer's first argument after `e` names, a `"…"`
/// string on the line, by its last path segment as ada names a file (`"a.json"`,
/// `"share/x/y.toml"`, `"${name}.py"`); [`SHEBANG`] for a name without an extension (or one
/// not written out: `name`, `(n + "-x")`); 0 for none. `.conf` names none: bat's set gives it
/// nginx's, but a writer's `.conf` is mostly another program's (Nix's, systemd's, fontconfig's
/// XML).
fn named(b: &[u8], e: usize) -> u8 {
    let q = blanks(b, e);
    match b.get(q) {
        Some(b'"') => {}
        Some(&c) if c == b'(' || CLASS[usize::from(c)] == C_NAME => return SHEBANG,
        _ => return 0,
    }
    let Some(r) = dq_end(b, q + 1) else {
        return 0;
    };
    name_slot(&b[q + 1..r - 1])
}

/// The slot in [`FILES`] of the language a file's name or path `name` names, as ada names a
/// file: its last path segment's whole name where it starts with a dot (`.zshrc`, X-157), else
/// its extension; [`SHEBANG`] for none, 0 for an extension or a dotfile no language in
/// [`FILES`] is named by.
fn name_slot(name: &[u8]) -> u8 {
    let seg = name.rsplit(|&c| c == b'/').next().unwrap_or(name);
    match seg.iter().rposition(|&c| c == b'.') {
        Some(0) => match ext_slot(seg) {
            0 => SHEBANG,
            s => s,
        },
        Some(i) => ext_slot(&seg[i + 1..]),
        None => SHEBANG,
    }
}

/// The file an attribute path `b[p..eq]` names (X-157), its last string part's name
/// ([`name_slot`]): a binding's of its `text` (`"a.json".text`, `etc."b/c.toml".text`), or a set's
/// whose `text` binding holds it ([`FL_SET`]: `"a.lua" = { … }`); 0 for none.
fn path_file(b: &[u8], mut p: usize, eq: usize) -> u8 {
    // the last string part's name, and whether the part after it is the last, or `text`
    let mut name: Option<&[u8]> = None;
    let mut at_name = false;
    let mut text = false;
    while p < eq {
        match b[p] {
            b'"' => {
                let Some(r) = dq_end(b, p + 1) else {
                    return 0;
                };
                name = Some(&b[p + 1..r - 1]);
                (at_name, text) = (true, false);
                p = r;
            }
            b'$' => {
                let Some(r) = braces_end(b, p + 2) else {
                    return 0;
                };
                (at_name, text) = (false, false);
                p = r;
            }
            c if CLASS[usize::from(c)] == C_NAME => {
                let e = name_end(b, p);
                text = at_name && &b[p..e] == b"text";
                at_name = false;
                p = e;
            }
            _ => return 0,
        }
        p = blanks(b, p);
        if b.get(p) == Some(&b'.') {
            p = blanks(b, p + 1);
        }
    }
    match name {
        Some(n) if at_name => match name_slot(n) {
            0 => 0,
            s => FL_SET | s,
        },
        Some(n) if text => name_slot(n),
        _ => 0,
    }
}

/// The extensions ada names a file of in [`FILES`]' languages, and the dotfiles it names by
/// their whole name (`EXTENSIONS` in `lang.rs`; `ext_slots_name_as_ada_does`), by slot.
const EXT_LIST: &[(&[u8], u8)] = &[
    (b"sh", BASH_SLOT),
    (b"bash", BASH_SLOT),
    (b"zsh", BASH_SLOT),
    (b"ash", BASH_SLOT),
    // dotfiles, by their whole name (X-157: `home.file.".zshrc".text`)
    (b".bash_aliases", BASH_SLOT),
    (b".bash_completions", BASH_SLOT),
    (b".bash_functions", BASH_SLOT),
    (b".bash_login", BASH_SLOT),
    (b".bash_logout", BASH_SLOT),
    (b".bash_profile", BASH_SLOT),
    (b".bash_variables", BASH_SLOT),
    (b".bashrc", BASH_SLOT),
    (b".profile", BASH_SLOT),
    (b".zlogin", BASH_SLOT),
    (b".zlogout", BASH_SLOT),
    (b".zprofile", BASH_SLOT),
    (b".zshenv", BASH_SLOT),
    (b".zshrc", BASH_SLOT),
    (b".editorconfig", INI_SLOT),
    (b".coveragerc", INI_SLOT),
    (b".pylintrc", INI_SLOT),
    (b".gitlint", INI_SLOT),
    (b".hgrc", INI_SLOT),
    (b"json", JSON_SLOT),
    (b"ipynb", JSON_SLOT),
    (b"toml", TOML_SLOT),
    (b"tml", TOML_SLOT),
    (b"ini", INI_SLOT),
    (b"inf", INI_SLOT),
    (b"reg", INI_SLOT),
    (b"lng", INI_SLOT),
    (b"cfg", INI_SLOT),
    (b"desktop", INI_SLOT),
    (b"url", INI_SLOT),
    (b"properties", PROPS_SLOT),
    (b"diff", DIFF_SLOT),
    (b"patch", DIFF_SLOT),
    (b"yaml", YAML_SLOT),
    (b"yml", YAML_SLOT),
    (b"sql", SQL_SLOT),
    (b"ddl", SQL_SLOT),
    (b"dml", SQL_SLOT),
    (b"lua", LUA_SLOT),
    (b"css", CSS_SLOT),
    (b"el", EL_SLOT),
    (b"scm", SCM_SLOT),
    (b"ss", SCM_SLOT),
    (b"sld", SCM_SLOT),
    (b"clj", CLJ_SLOT),
    (b"cljc", CLJ_SLOT),
    (b"cljs", CLJ_SLOT),
    (b"edn", CLJ_SLOT),
    (b"c", C_SLOT),
    (b"cpp", CPP_SLOT),
    (b"cc", CPP_SLOT),
    (b"cp", CPP_SLOT),
    (b"cxx", CPP_SLOT),
    (b"c++", CPP_SLOT),
    (b"hh", CPP_SLOT),
    (b"hpp", CPP_SLOT),
    (b"hxx", CPP_SLOT),
    (b"h++", CPP_SLOT),
    (b"inl", CPP_SLOT),
    (b"ipp", CPP_SLOT),
    (b"hs", HS_SLOT),
    (b"rs", RS_SLOT),
    (b"js", JS_SLOT),
    (b"mjs", JS_SLOT),
    (b"jsx", JS_SLOT),
    (b"babel", JS_SLOT),
    (b"es6", JS_SLOT),
    (b"cjs", JS_SLOT),
    (b"xml", XML_SLOT),
    (b"xsd", XML_SLOT),
    (b"xslt", XML_SLOT),
    (b"tld", XML_SLOT),
    (b"dtml", XML_SLOT),
    (b"rng", XML_SLOT),
    (b"rss", XML_SLOT),
    (b"opml", XML_SLOT),
    (b"svg", XML_SLOT),
    (b"xaml", XML_SLOT),
    (b"php", PHP_SLOT),
    (b"php3", PHP_SLOT),
    (b"php4", PHP_SLOT),
    (b"php5", PHP_SLOT),
    (b"php7", PHP_SLOT),
    (b"phps", PHP_SLOT),
    (b"phpt", PHP_SLOT),
    (b"phtml", PHP_SLOT),
    (b"py", PY_SLOT),
    (b"py3", PY_SLOT),
    (b"pyw", PY_SLOT),
    (b"pyi", PY_SLOT),
    (b"pyx", PY_SLOT),
    (b"pxd", PY_SLOT),
    (b"pxi", PY_SLOT),
    (b"rpy", PY_SLOT),
    (b"cpy", PY_SLOT),
    (b"gyp", PY_SLOT),
    (b"gypi", PY_SLOT),
    (b"vpy", PY_SLOT),
    (b"bzl", PY_SLOT),
    (b"bazel", PY_SLOT),
];
const NE: usize = EXT_LIST.len();
static EXT_TEXT: [&[u8]; NE] = {
    let mut t: [&[u8]; NE] = [b""; NE];
    let mut i = 0;
    while i < NE {
        t[i] = EXT_LIST[i].0;
        i += 1;
    }
    t
};
static EXTS: Words<256, NE, { text_len(&EXT_TEXT) }> = Words::new(&EXT_TEXT);
/// Each extension's slot by its id in [`EXTS`].
static EXT_SLOTS: [u8; NE + 1] = {
    let mut t = [0; NE + 1];
    let mut i = 0;
    while i < NE {
        t[i + 1] = EXT_LIST[i].1;
        i += 1;
    }
    t
};

/// The slot in [`FILES`] of the language ada names a file of extension `ext` (or a dotfile of
/// that name) in, 0 for none.
fn ext_slot(ext: &[u8]) -> u8 {
    let mut l = [0u8; 24];
    let Some(l) = l.get_mut(..ext.len()) else {
        return 0;
    };
    l.copy_from_slice(ext);
    l.make_ascii_lowercase();
    EXTS.value(l, &EXT_SLOTS)
}

/// Whether the `"` at `p` follows a writer's name on its line, so it is the writer's first
/// argument, a name, not its script: a shell writer's ([`W_SHELL`]) or another's (`write…`,
/// `toFile`).
fn writer_before(b: &[u8], p: usize) -> bool {
    let mut e = p;
    while e > 0 && is_blank(b[e - 1]) {
        e -= 1;
    }
    let mut s = e;
    while s > 0 && NAME_BYTE[usize::from(b[s - 1])] {
        s -= 1;
    }
    if s == e || CLASS[usize::from(b[s])] != C_NAME {
        return false;
    }
    let w = &b[s..e];
    WORDS.value(w, &INFO) & W_SHELL != 0
        || (w.len() > 5 && w.starts_with(b"write") && w[5].is_ascii_uppercase())
        || w == b"toFile"
}

/// Whether the `''` at `p` is a writer's call's string (X-156): a writer's name before it on its
/// line ([`writer_before`]), its `"…"` name and a set between (`writeText "x" ''`,
/// `writePython3 "y" { } ''`), within [`PART`] bytes. In a file's interpolation such a string
/// is no text spliced into the file.
fn writer_string(b: &[u8], p: usize) -> bool {
    let lo = p.saturating_sub(PART);
    let back = |mut e: usize| {
        while e > lo && is_blank(b[e - 1]) {
            e -= 1;
        }
        e
    };
    let mut e = back(p);
    if e > lo && b[e - 1] == b'}' {
        // the set's `{`
        let mut d = 0u32;
        loop {
            if e == lo {
                return false;
            }
            e -= 1;
            match b[e] {
                b'}' => d += 1,
                b'{' => {
                    d -= 1;
                    if d == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        e = back(e);
    }
    if e > lo && b[e - 1] == b'"' {
        // the name's opening quote
        e -= 1;
        loop {
            if e == lo {
                return false;
            }
            e -= 1;
            if b[e] == b'"' && (e == 0 || b[e - 1] != b'\\') {
                break;
            }
        }
    }
    writer_before(b, e)
}

/// Whether the `"` at `p` follows a name on its line: a function's argument (`text =
/// concatStringsSep "\n" …`), no writer's file's string (X-157).
fn after_name(b: &[u8], p: usize) -> bool {
    let mut e = p;
    while e > 0 && is_blank(b[e - 1]) {
        e -= 1;
    }
    e > 0 && NAME_BYTE[usize::from(b[e - 1])]
}

/// Whether a shell string is open in frames `f`, `depth` levels open (those kept).
#[inline(always)]
fn sh_in(f: u128, depth: u8) -> bool {
    // a nibble of four ones, among the levels open
    let ones = f & f >> 1 & f >> 2 & f >> 3 & 0x1111_1111_1111_1111_1111_1111_1111_1111;
    let n = u32::from(depth.min(32));
    let open = if n == 32 {
        u128::MAX
    } else {
        (1u128 << (4 * n)) - 1
    };
    ones & open != 0
}

/// Whether `$` at `p` in a string's text opens an interpolation: `${`, after an even run of
/// `$`s (`$$` is two dollars, so `$${` is text).
fn interp_in_text(b: &[u8], p: usize) -> bool {
    if b[p] != b'$' || b.get(p + 1) != Some(&b'{') {
        return false;
    }
    let mut i = p;
    while i > 0 && b[i - 1] == b'$' {
        i -= 1;
    }
    (p - i).is_multiple_of(2)
}

/// [`Scan::view`]'s reading of the line's shell text from `q`, the frames `f` above the
/// outermost shell string's (`rel` levels, it among them; a `"…"` string where `dq`) followed
/// as the scan follows them, into `v`'s cooked text and its runs of the line's bytes. Ends at
/// the line's end, the outermost string's close, a `#` comment in code, or past the levels the
/// scan keeps. A writer's file's ([`ShellView::file`]): no string spliced from an
/// interpolation, `\n` in a `"…"` string a line break ([`ShellView::breaks`]).
fn read_shell(b: &[u8], mut q: usize, mut f: u128, mut rel: usize, dq: bool, v: &mut ShellView) {
    let n = b.len();
    let seg = |v: &mut ShellView, s: usize, e: usize| {
        if e > s {
            v.segs
                .push((v.cooked.len() as u32, s as u32, (e - s) as u32));
            v.cooked.extend_from_slice(&b[s..e]);
        }
    };
    // the level a writer's call whose string isn't shell holds (as the scan's hold), 0 for none
    let mut hold = 0;
    macro_rules! push {
        ($frame:expr) => {{
            if rel >= 32 {
                return;
            }
            f = f << 4 | u128::from($frame);
            rel += 1;
        }};
    }
    macro_rules! pop {
        () => {{
            f >>= 4;
            rel -= 1;
            if rel == 0 {
                return;
            }
            if rel < hold {
                hold = 0;
            }
        }};
    }
    let top = |f: u128| (f & 15) as u8;
    while q < n {
        match top(f) {
            F_SH if dq && rel == 1 => {
                // the outermost string, a `"…"` one: its escapes cooked
                let mut s = q;
                loop {
                    let r = find_any(b, q, *b"\"\\$");
                    if r >= n {
                        seg(v, s, n);
                        return;
                    }
                    match b[r] {
                        b'"' => {
                            seg(v, s, r);
                            return;
                        }
                        b'\\' => {
                            seg(v, s, r);
                            let c = b.get(r + 1).copied();
                            let e = (r + 1 + c.map_or(0, char_len)).min(n);
                            match c {
                                // a line break in the script: its command ends
                                Some(b'n') if v.file == 0 => v.cooked.push(b';'),
                                // a file's line break: its language's next line
                                Some(b'n') => v.breaks.push(v.cooked.len() as u32),
                                Some(b'r') => v.cooked.push(b' '),
                                Some(b't') => v.cooked.push(b'\t'),
                                _ => v.cooked.extend_from_slice(&b[r + 1..e]),
                            }
                            q = e;
                            s = q;
                        }
                        _ => match b.get(r + 1) {
                            Some(b'{') => {
                                seg(v, s, r);
                                push!(F_INTERP);
                                v.islands.push((rel as u32, v.cooked.len() as u32));
                                q = r + 2;
                                break;
                            }
                            Some(b'$') => q = r + 2,
                            _ => q = r + 1,
                        },
                    }
                }
            }
            F_SH => {
                let mut s = q;
                loop {
                    let r = find_any(b, q, *b"'$");
                    if r >= n {
                        seg(v, s, n);
                        return;
                    }
                    if b[r] == b'\'' {
                        if b.get(r + 1) != Some(&b'\'') {
                            q = r + 1;
                            continue;
                        }
                        seg(v, s, r);
                        match b.get(r + 2) {
                            Some(b'$') => {
                                v.cooked.push(b'$');
                                q = r + 3;
                            }
                            Some(b'\'') => {
                                v.cooked.extend_from_slice(b"''");
                                q = r + 3;
                            }
                            Some(b'\\') => {
                                let c = b.get(r + 3).copied();
                                let e = (r + 3 + c.map_or(0, char_len)).min(n);
                                match c {
                                    Some(b'n' | b'r') => v.cooked.push(b' '),
                                    Some(b't') => v.cooked.push(b'\t'),
                                    _ => v.cooked.extend_from_slice(&b[r + 3..e]),
                                }
                                q = e;
                            }
                            _ => {
                                q = r + 2;
                                pop!();
                                break;
                            }
                        }
                        s = q;
                        continue;
                    }
                    match b.get(r + 1) {
                        Some(b'{') => {
                            seg(v, s, r);
                            push!(F_INTERP);
                            v.islands.push((rel as u32, v.cooked.len() as u32));
                            q = r + 2;
                            break;
                        }
                        Some(b'$') => q = r + 2,
                        _ => q = r + 1,
                    }
                }
            }
            F_DQ => {
                let r = find_any(b, q, *b"\"\\$");
                if r >= n {
                    return;
                }
                q = match b[r] {
                    b'"' => {
                        pop!();
                        r + 1
                    }
                    b'\\' => (r + 1 + b.get(r + 1).map_or(0, |&c| char_len(c))).min(n),
                    _ => match b.get(r + 1) {
                        Some(b'{') => {
                            push!(F_INTERP);
                            r + 2
                        }
                        Some(b'$') => r + 2,
                        _ => r + 1,
                    },
                };
            }
            F_IND => {
                let r = find_any(b, q, *b"'$");
                if r >= n {
                    return;
                }
                q = if b[r] == b'\'' {
                    match (b.get(r + 1), b.get(r + 2)) {
                        (Some(b'\''), Some(b'$' | b'\'')) => r + 3,
                        (Some(b'\''), Some(b'\\')) => {
                            (r + 3 + b.get(r + 3).map_or(0, |&c| char_len(c))).min(n)
                        }
                        (Some(b'\''), _) => {
                            pop!();
                            r + 2
                        }
                        _ => r + 1,
                    }
                } else {
                    match b.get(r + 1) {
                        Some(b'{') => {
                            push!(F_INTERP);
                            r + 2
                        }
                        Some(b'$') => r + 2,
                        _ => r + 1,
                    }
                };
            }
            F_COMMENT => {
                let Some(r) = b[q..].windows(2).position(|w| w == b"*/") else {
                    return;
                };
                q += r + 2;
                pop!();
            }
            F_PATH => {
                // a path's rest after its interpolation, as `path_rest` reads it
                while q < n && (PATH_BYTE[usize::from(b[q])] || b[q] == b'/') {
                    q += 1;
                }
                if interp_at(b, q) {
                    push!(F_INTERP);
                    q += 2;
                } else {
                    pop!();
                }
            }
            _ => {
                let c = b[q];
                q = match CLASS[usize::from(c)] {
                    C_NAME => {
                        let e = name_end(b, q);
                        if hold == 0
                            && matches!(c, b'w' | b't')
                            && held_writer(b, e, WORDS.value(&b[q..e], &INFO))
                        {
                            hold = rel;
                        }
                        e
                    }
                    C_DQUOTE => {
                        push!(F_DQ);
                        q + 1
                    }
                    C_SQUOTE if b.get(q + 1) == Some(&b'\'') => {
                        // a string in the script's interpolation: its text spliced in, but a
                        // writer's whose string isn't shell; in a file's, but a writer's own
                        // (X-156: [`writer_string`], as the scan reads it)
                        let own = if v.file == 0 {
                            hold != 0
                        } else {
                            writer_string(b, q)
                        };
                        if own {
                            push!(F_IND);
                        } else {
                            push!(F_SH);
                        }
                        q + 2
                    }
                    C_HASH => return,
                    C_SLASH if b.get(q + 1) == Some(&b'*') => {
                        push!(F_COMMENT);
                        q + 2
                    }
                    C_LBRACE => {
                        push!(F_BRACE);
                        q + 1
                    }
                    C_LPAREN => {
                        push!(F_PARENS);
                        q + 1
                    }
                    C_LBRACKET => {
                        push!(F_LIST);
                        q + 1
                    }
                    C_RBRACE => {
                        while matches!(top(f), F_WITH | F_INHERIT | F_LET) {
                            pop!();
                        }
                        match top(f) {
                            F_INTERP => {
                                // an interpolation without shell text in it: a word to bash
                                if let Some(&(l, c)) = v.islands.last()
                                    && l as usize == rel
                                {
                                    v.islands.pop();
                                    if v.cooked.len() == c as usize {
                                        v.cooked.push(if v.file == 0 { b'x' } else { v.file });
                                    }
                                }
                                pop!();
                            }
                            F_BRACE | F_FORMALS => pop!(),
                            _ => {}
                        }
                        q + 1
                    }
                    C_RPAREN => {
                        while matches!(top(f), F_WITH | F_INHERIT | F_LET) {
                            pop!();
                        }
                        if matches!(top(f), F_PARENS | F_INHERIT_PARENS) {
                            pop!();
                        }
                        q + 1
                    }
                    C_RBRACKET => {
                        while matches!(top(f), F_WITH | F_INHERIT | F_LET) {
                            pop!();
                        }
                        if top(f) == F_LIST {
                            pop!();
                        }
                        q + 1
                    }
                    C_SEMI => {
                        if matches!(top(f), F_WITH | F_INHERIT) {
                            pop!();
                        } else if hold == rel {
                            hold = 0;
                        }
                        q + 1
                    }
                    C_DOLLAR if b.get(q + 1) == Some(&b'{') => {
                        push!(F_INTERP);
                        q + 2
                    }
                    C_WIDE => q + char_len(c),
                    _ => q + 1,
                };
            }
        }
    }
}

/// Whether the `.` at `p` follows the name `builtins`.
#[inline(always)]
fn after_builtins(b: &[u8], p: usize) -> bool {
    p >= 8
        && b[p - 1] == b's'
        && u64::from_le_bytes(b[p - 8..p].try_into().unwrap_or([0; 8]))
            == u64::from_le_bytes(*b"builtins")
        && (p == 8 || !NAME_BYTE[usize::from(b[p - 9])] && b[p - 9] != b'.')
}

/// Where a remark at `q` ends (before `e`): `TODO`, `FIXME`, `BUG` as words, `!!!`; `q` for
/// none.
#[inline(never)]
fn remark_end(b: &[u8], q: usize, e: usize) -> usize {
    let word = |w: &[u8]| {
        let r = q + w.len();
        r <= e
            && &b[q..r] == w
            && (q == 0 || !NAME_BYTE[usize::from(b[q - 1])])
            && (r == b.len() || !NAME_BYTE[usize::from(b[r])])
    };
    let w: &[u8] = match b[q] {
        b'T' => b"TODO",
        b'F' => b"FIXME",
        b'B' => b"BUG",
        _ => {
            return if q + 3 <= e && &b[q..q + 3] == b"!!!" {
                q + 3
            } else {
                q
            };
        }
    };
    if word(w) { q + w.len() } else { q }
}

/// Where a float's exponent at `p` ends (`e`, an optional sign, digits); `p` for none.
#[inline(always)]
fn exponent(b: &[u8], p: usize) -> usize {
    if !matches!(b.get(p), Some(b'e' | b'E')) {
        return p;
    }
    let s = if matches!(b.get(p + 1), Some(b'+' | b'-')) {
        p + 2
    } else {
        p + 1
    };
    if !b.get(s).is_some_and(u8::is_ascii_digit) {
        return p;
    }
    let mut e = s + 1;
    while e < b.len() && b[e].is_ascii_digit() {
        e += 1;
    }
    e
}

/// How far an attribute path's string or interpolation is looked for its end.
const PART: usize = 256;

/// Where a `"…"` string whose text starts at `p` ends, past its `"`, within [`PART`] bytes;
/// `None` for one that runs on.
fn dq_end(b: &[u8], mut p: usize) -> Option<usize> {
    let end = (p + PART).min(b.len());
    loop {
        p = find_in(b, p, end, *b"\"\\");
        if p >= end {
            return None;
        }
        match b[p] {
            b'"' => return Some(p + 1),
            _ => p += 2,
        }
    }
}

/// Where `${…}`'s braces whose inside starts at `p` close, past the `}`, within [`PART`] bytes
/// (braces counted, strings' braces too); `None` for ones that run on.
fn braces_end(b: &[u8], mut p: usize) -> Option<usize> {
    let end = (p + PART).min(b.len());
    let mut depth = 1u32;
    loop {
        p = find_in(b, p, end, *b"{}");
        if p >= end {
            return None;
        }
        match b[p] {
            b'{' => depth += 1,
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some(p + 1);
                }
            }
        }
        p += 1;
    }
}

#[cfg(test)]
mod tests;
