//! Makefiles (ADR-40), on a scanner of ada's own (X-097, ADR-41): the floor probe's shape, grown
//! with bat's corners. bat's `Makefile` is the reference for scopes and the parity oracle, not the
//! design; the shell in recipes, `!=`, `$(shell …)` and `bash -c '…'` is bash's scanner
//! ([`super::line_spans`], X-094).
//!
//! **A line is classified once**, at its start, by what the frames left open say it is (a recipe
//! line under a rule, a `define`'s body, a continued construct) or else by `main`'s reading of
//! it: a comment, a directive (`ifeq`, `else`, `endif`, `include`, `vpath`, `define`,
//! `export`, `override`), an assignment or a rule, the leftmost of them as bat's patterns
//! would find it, ties to the grammar's first ([`Scan::classify`]; `just_eat`'s lookaheads in
//! [`Scan::var_ahead`] and [`Scan::rule_ahead`], over the line's parenthesis groups found once,
//! [`Parens`]). The commonest lines, an assignment or a rule, are found first, and the other
//! patterns searched only as far as they could still start before it.
//!
//! **The rest of the line is one loop** over its bytes on a small stack of frames: a construct's
//! own (a variable's name, a value, a rule's targets and prerequisites, a directive's arguments, a
//! comment) and the ones that nest (`$(…)`, `${…}`, a function's arguments, strings, groups).
//! Each frame's stop bytes are one class table ([`STOP`]); the text between them is the frame's
//! kind, written as the loop passes it.
//!
//! **A shell region is a frame** ([`SH_RECIPE`] and the others), where bat sets
//! `scope:source.shell` with make's patterns in front (X-094). Its text up to where the region
//! ends (a line's end, the `)` closing `$(shell …)`, an embed's quote) is read for make's tokens
//! first: `$(…)`, `${…}`, `$$`, `$@` and the like everywhere, and in `$(shell …)` make's strings
//! and parentheses too. Each runs on make's frames above the region's. bash's scanner then lexes
//! the region's text in one call, from the state the region kept, **reading it as make hands it
//! to the shell**: a token stands as a word (`_`), `$$` as `$` ([`Cuts::stand_in`]), and its spans
//! are laid back around the tokens' ([`Scan::lay`]), a token's text that no make scope takes in
//! bash's kind at its byte (a string's, a command's name, an option's). So `$(CC) -c` reads `-c`
//! as an option and `$$f` as a shell variable, where bat's shell never saw the token (a command
//! `-c`, a plain `f`): departures by design. Where a token starts a word that goes on
//! (`$(Q)echo`), or precedes an assignment (`$(ENV) CC=gcc`), it stands as nothing, as in bat.
//! Where bash's reading differs from bat's machine (a `#` that bash reads inside a string, a
//! `\` in single quotes), bash's reading stands (ADR-41).
//!
//! **Kinds are leaves**, as bash's are: bash's kinds below `0x100`, make's from there
//! ([`MK`]). In the 14 built-in themes a byte's rule is that of its deepest scope that isn't
//! `meta.*` or `source.*`, but where a `variable.parameter` scope around it outranks a
//! `variable.*` leaf: an option's ([`super::TWIN`]: a make token inside a shell option) or a
//! make substitution's ([`TWIN_MK`]).
//!
//! Between lines the state holds the frames, a byte each, their count in `arg`, and above them
//! bash's state for the topmost shell region open past the line (a recipe's `\`, an open
//! `$(shell …)`).

use std::mem::MaybeUninit;

use crate::prim::{char_len, find_in};
use crate::stack::HELD;
use crate::{Out, State};

/// No kind: plain text.
const PLAIN: u16 = u16::MAX;

/// A make kind is this plus its index in [`MK_KINDS`].
pub const MK: u16 = 0x100;
/// A `variable.*` kind inside a make substitution is this plus the kind.
pub const TWIN_MK: u16 = 0x400;

macro_rules! make_kinds {
    ($($k:ident = $s:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(u16)]
        enum MkId { $($k,)* Count }
        /// The make kinds, each a stack of scopes under `source.makefile`.
        #[allow(dead_code)]
        mod mk {
            $(pub const $k: u16 = super::MK + super::MkId::$k as u16;)*
        }
        static MK_KINDS: [&str; MkId::Count as usize] = [$($s,)*];
    };
}

make_kinds! {
    STR_SINGLE = "meta.string.makefile string.quoted.single.makefile",
    STR_DOUBLE = "meta.string.makefile string.quoted.double.makefile",
    STR_BEGIN = "punctuation.definition.string.begin.makefile",
    STR_END = "punctuation.definition.string.end.makefile",
    GROUP_BEGIN = "punctuation.section.group.begin.makefile",
    GROUP_END = "punctuation.section.group.end.makefile",
    SEPARATOR = "punctuation.separator.makefile",
    STRAY_PAREN = "invalid.illegal.stray.paren.makefile",
    VPATH = "keyword.control.vpath.makefile",
    UNQUOTED = "meta.string.makefile string.unquoted.makefile",
    PREREQ = "meta.function.arguments.makefile string.unquoted.makefile",
    IMPORT = "keyword.control.import.makefile",
    CONDITIONAL = "keyword.control.conditional.makefile",
    PERCENT = "variable.language.makefile",
    WILDCARD = "variable.language.wildcard.makefile",
    TARGET = "meta.function.makefile entity.name.function.makefile",
    ASSIGN = "keyword.operator.assignment.makefile",
    COMMENT_MARK = "comment.line.number-sign.makefile punctuation.definition.comment.makefile",
    COMMENT = "comment.line.number-sign.makefile",
    TERMINATOR = "punctuation.terminator.makefile",
    CONST_LANG = "constant.language.makefile",
    EXPECTED_SPACES = "invalid.illegal.inconsistent.expected.spaces.makefile",
    EXPECTED_TAB = "invalid.illegal.inconsistent.expected.tab.makefile",
    CONTINUATION = "punctuation.separator.continuation.line.makefile",
    ESCAPE = "constant.character.escape.makefile",
    BLOCK_END = "keyword.other.block.end.makefile",
    BLOCK_BEGIN = "keyword.other.block.begin.makefile",
    CALL_LANG = "meta.function-call.makefile constant.language.call.makefile",
    VAR_FUNCTION = "meta.function-call.makefile variable.function.makefile",
    BUILTIN = "support.function.builtin.makefile",
    CONTROL = "keyword.control.makefile",
    VAR_OTHER = "variable.other.makefile",
    INTERP_BEGIN = "meta.interpolation.makefile punctuation.section.interpolation.begin.makefile",
    INTERP_END = "meta.interpolation.makefile punctuation.section.interpolation.end.makefile",
    SUBST_MARK = "punctuation.definition.substitution.makefile",
    ASSIGN_MARK = "punctuation.definition.assignment.makefile",
    VAR_PARAM = "variable.parameter.makefile",
    AUTOMATIC = "variable.language.automatic.makefile",
    SINGLE_CHAR = "keyword.other.single-character-variable.makefile",
    STRAY_ENDEF = "invalid.illegal.stray.endef.makefile",
}

/// The scopes under `source.makefile` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let mut out = Vec::new();
    if kind & TWIN_MK != 0 {
        out.push("variable.parameter.makefile");
    }
    if kind & super::TWIN != 0 {
        out.push("variable.parameter.option.shell");
    }
    let k = kind & (MK | (MK - 1));
    let s = if k < MK {
        super::KINDS.get(usize::from(k))
    } else {
        MK_KINDS.get(usize::from(k - MK))
    };
    match s {
        Some(s) => out.extend(s.split(' ')),
        None => out.clear(),
    }
    out
}

/// Whether a make kind's deepest scope is a `variable.*` one a `variable.parameter` scope
/// around it outranks.
pub fn is_variable(kind: u16) -> bool {
    matches!(
        kind,
        mk::PERCENT | mk::WILDCARD | mk::VAR_FUNCTION | mk::VAR_OTHER | mk::AUTOMATIC
    )
}

macro_rules! frames {
    ($($f:ident,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u8)]
        enum FrameId { $($f,)* Count }
        $(const $f: u8 = FrameId::$f as u8;)*
        const FRAMES: usize = FrameId::Count as usize;
    };
}

// bat's contexts where a frame stands for one (by its name in the grammar)
frames! {
    // `main`
    MAIN,
    // a construct continued past a line's `\`, to the next line's first non-blank
    CONT,
    // a make comment, a shell comment in a recipe, a rule's trailing comment
    COMMENT,
    SH_COMMENT,
    RULE_COMMENT,
    // `ifeq (…)`'s arguments, the same in a recipe block, strings and groups in them
    COND,
    R_IF,
    STR_S,
    STR_D,
    GROUP,
    VPATH,
    VPATH_DIRS,
    INCLUDE,
    ELSE,
    ENDIF,
    // a rule: its targets, a target-specific variable's definition, its prerequisites, a recipe
    // after `;`, and the lines after it
    TARGET,
    TVAR,
    PREREQ,
    INLINE,
    JUNCTION,
    R_TABS,
    R_SPACES,
    // function calls: `$(name …)`'s arguments by kind, `$(call NAME, …)`'s name, a group
    FCALL,
    CALL,
    CALL_PAT,
    CALL_WILD,
    CALL_INFO,
    CALL_SHELL,
    BAL,
    // definitions
    OVERRIDE,
    EXPORT,
    VARIABLE,
    MAYBE_SHELL,
    VALUE,
    DEFINE,
    DEFINE_VALUE,
    DEFINE_NESTED,
    // substitutions
    SUBST_P,
    SUBST_B,
    // `bash -c '…'`'s closing quote
    EMBED_SQ,
    EMBED_DQ,
    EMBED_BT,
    // shell regions, last: a recipe's commands, `$(shell …)`, `!=`'s, `bash -c`'s by its quote
    SH_RECIPE,
    SH_CALL,
    SH_ASSIGN,
    SH_SQ,
    SH_DQ,
    SH_BT,
}

/// Per frame, the kind of the text in it no token takes, where it has one of its own (else the
/// frame below's).
static BASE: [u16; FRAMES] = {
    let mut t = [PLAIN; FRAMES];
    macro_rules! set {
        ($($f:ident: $k:expr;)*) => {
            $(t[$f as usize] = $k;)*
        };
    }
    set! {
        COMMENT: mk::COMMENT;
        SH_COMMENT: super::k::COMMENT;
        RULE_COMMENT: mk::COMMENT;
        STR_S: mk::STR_SINGLE;
        STR_D: mk::STR_DOUBLE;
        VPATH_DIRS: mk::UNQUOTED;
        INCLUDE: mk::UNQUOTED;
        TARGET: mk::TARGET;
        PREREQ: mk::PREREQ;
        CALL: mk::VAR_FUNCTION;
        EXPORT: mk::VAR_OTHER;
        VARIABLE: mk::VAR_OTHER;
        VALUE: mk::UNQUOTED;
        DEFINE: mk::VAR_OTHER;
        DEFINE_VALUE: mk::UNQUOTED;
        SUBST_P: mk::VAR_PARAM;
        SUBST_B: mk::VAR_PARAM;
    }
    t
};

// ---- byte classes: a bit each; a frame's stops are their union

const C_DOLLAR: u32 = 1;
const C_BSLASH: u32 = 1 << 1;
const C_HASH: u32 = 1 << 2;
const C_PERCENT: u32 = 1 << 3;
const C_STAR: u32 = 1 << 4;
const C_OPEN: u32 = 1 << 5;
const C_CLOSE: u32 = 1 << 6;
const C_BRACE: u32 = 1 << 7;
const C_COMMA: u32 = 1 << 8;
const C_COLON: u32 = 1 << 9;
const C_EQ: u32 = 1 << 10;
const C_SEMI: u32 = 1 << 11;
const C_SQ: u32 = 1 << 12;
const C_DQ: u32 = 1 << 13;
const C_BT: u32 = 1 << 14;
/// `\s` but the line's end
const C_WS: u32 = 1 << 15;
/// `i`: `ifeq` and its kin after `else`
const C_I: u32 = 1 << 16;

static CLS: [u32; 256] = {
    let mut t = [0u32; 256];
    t[b'$' as usize] = C_DOLLAR;
    t[b'\\' as usize] = C_BSLASH;
    t[b'#' as usize] = C_HASH;
    t[b'%' as usize] = C_PERCENT;
    t[b'*' as usize] = C_STAR;
    t[b'(' as usize] = C_OPEN;
    t[b')' as usize] = C_CLOSE;
    t[b'}' as usize] = C_BRACE;
    t[b',' as usize] = C_COMMA;
    t[b':' as usize] = C_COLON;
    t[b'=' as usize] = C_EQ;
    t[b';' as usize] = C_SEMI;
    t[b'\'' as usize] = C_SQ;
    t[b'"' as usize] = C_DQ;
    t[b'`' as usize] = C_BT;
    t[b' ' as usize] = C_WS;
    t[b'\t' as usize] = C_WS;
    t[b'\r' as usize] = C_WS;
    t[0x0b] = C_WS;
    t[0x0c] = C_WS;
    t[b'i' as usize] = C_I;
    t
};

/// Per frame, the bytes its loop stops at.
static STOP: [u32; FRAMES] = {
    let mut t = [0u32; FRAMES];
    const CALL_ARGS: u32 = C_CLOSE | C_OPEN | C_COMMA | C_BSLASH | C_SQ | C_DQ | C_DOLLAR;
    macro_rules! set {
        ($($f:ident: $k:expr;)*) => {
            $(t[$f as usize] = $k;)*
        };
    }
    set! {
        COND: C_SQ | C_DQ | C_OPEN | C_CLOSE | C_BSLASH;
        R_IF: C_SQ | C_DQ | C_OPEN | C_CLOSE | C_BSLASH | C_HASH;
        STR_S: C_SQ | C_BSLASH | C_DOLLAR;
        STR_D: C_DQ | C_BSLASH | C_DOLLAR;
        GROUP: C_CLOSE | C_DOLLAR | C_COMMA;
        VPATH: C_STAR | C_WS;
        INCLUDE: C_BSLASH | C_DOLLAR | C_HASH;
        ELSE: C_HASH | C_I;
        ENDIF: C_HASH;
        TARGET: C_BSLASH | C_DOLLAR | C_PERCENT | C_COLON;
        PREREQ: C_BSLASH | C_DOLLAR | C_PERCENT | C_HASH | C_SEMI;
        FCALL: CALL_ARGS;
        CALL_PAT: CALL_ARGS | C_PERCENT;
        CALL_WILD: CALL_ARGS | C_STAR;
        CALL_INFO: C_CLOSE | C_OPEN | C_COMMA | C_DOLLAR;
        CALL: C_CLOSE | C_COMMA | C_DOLLAR;
        BAL: C_CLOSE | C_DOLLAR;
        EXPORT: C_BSLASH | C_DOLLAR | C_EQ;
        VARIABLE: C_DOLLAR | C_EQ;
        VALUE: C_BSLASH | C_HASH | C_DOLLAR;
        DEFINE: C_DOLLAR;
        SUBST_P: C_CLOSE | C_COLON | C_EQ | C_PERCENT | C_DOLLAR;
        SUBST_B: C_BRACE | C_COLON | C_EQ | C_PERCENT | C_DOLLAR;
        EMBED_SQ: C_SQ;
        EMBED_DQ: C_DQ;
        EMBED_BT: C_BT;
    }
    t
};

/// Per byte, the shell regions a make token or the region's end may start on, a bit each
/// (`1 << (id - SH_RECIPE)`): a `$` in every region but an embed's, `\` (bash's escape, which
/// takes the byte after it from make, as bat's prototype does) where bash's prototype comes
/// before make's patterns, `)` ending `$(shell …)` and `!=`'s, make's strings and parentheses
/// in `$(shell …)`, an embed's quote.
static TRIGGER: [u8; 256] = {
    const R: u8 = 1;
    const C: u8 = 1 << (SH_CALL - SH_RECIPE);
    const A: u8 = 1 << (SH_ASSIGN - SH_RECIPE);
    let mut t = [0u8; 256];
    t[b'$' as usize] = R | C | A;
    t[b'\\' as usize] = R | C | A;
    t[b')' as usize] = C | A;
    t[b'(' as usize] = C;
    t[b'"' as usize] = C | 1 << (SH_DQ - SH_RECIPE);
    t[b'\'' as usize] = C | 1 << (SH_SQ - SH_RECIPE);
    t[b'`' as usize] = 1 << (SH_BT - SH_RECIPE);
    t
};

/// The function names a `$(` takes before a blank, by the frame their arguments open: `call`,
/// `patsubst` and `filter`, `wildcard`, `info` and kin, the rest, `shell`.
const CALLS: [(&[u8], u8); 37] = [
    (b"call", CALL),
    (b"patsubst", CALL_PAT),
    (b"filter", CALL_PAT),
    (b"wildcard", CALL_WILD),
    (b"info", CALL_INFO),
    (b"warning", CALL_INFO),
    (b"error", CALL_INFO),
    (b"subst", FCALL),
    (b"strip", FCALL),
    (b"findstring", FCALL),
    (b"filter-out", FCALL),
    (b"sort", FCALL),
    (b"word", FCALL),
    (b"wordlist", FCALL),
    (b"words", FCALL),
    (b"firstword", FCALL),
    (b"lastword", FCALL),
    (b"dir", FCALL),
    (b"notdir", FCALL),
    (b"suffix", FCALL),
    (b"basename", FCALL),
    (b"addsuffix", FCALL),
    (b"addprefix", FCALL),
    (b"join", FCALL),
    (b"realpath", FCALL),
    (b"abspath", FCALL),
    (b"if", FCALL),
    (b"or", FCALL),
    (b"and", FCALL),
    (b"foreach", FCALL),
    (b"file", FCALL),
    (b"value", FCALL),
    (b"eval", FCALL),
    (b"origin", FCALL),
    (b"flavor", FCALL),
    (b"guile", FCALL),
    (b"shell", CALL_SHELL),
];

/// `\s`, in ASCII.
fn ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Whether bash's kind is an option's: a make token right after it is inside the option.
fn in_option(kind: u16) -> bool {
    kind != PLAIN && kind & super::TWIN != 0
        || matches!(
            kind,
            super::k::OPTION | super::k::OPTION_MARK | super::k::OPTION_ASSIGN
        )
}

// ---- the parenthesis groups `just_eat` takes

/// The paren structure behind bat's `just_eat`: up to four levels of groups, each level
/// allowing only so many groups of so many shapes. Every `(` that closes gets its `)` and
/// whether the group, as level 1, is a shape `just_eat` takes; such a group is matched whole
/// or not at all, so the lookaheads built on it read runs of text between parens.
struct Parens {
    /// `(open, close, fits)` by `open`.
    g: [(u32, u32, bool); 32],
    n: usize,
}

impl Parens {
    fn new(b: &[u8]) -> Self {
        /// An open group: where, and what its groups so far allow.
        #[derive(Clone, Copy, Default)]
        struct Open {
            at: u32,
            kids: u8,
            k0_leaf: bool,
            k0_g2a: bool,
            k0_g2b: bool,
            k1_g2b: bool,
            all_g3a: bool,
            all_g3b: bool,
        }
        let mut p = Parens {
            g: [(0, 0, false); 32],
            n: 0,
        };
        let mut stack = [Open::default(); 32];
        let mut depth = 0;
        let mut i = 0;
        loop {
            i = find_in(b, i, b.len(), *b"()");
            if i >= b.len() {
                break;
            }
            if b[i] == b'(' {
                if depth < stack.len() {
                    stack[depth] = Open {
                        at: i as u32,
                        all_g3a: true,
                        all_g3b: true,
                        ..Open::default()
                    };
                }
                depth += 1;
            } else if depth > 0 {
                depth -= 1;
                if depth < stack.len() {
                    let o = stack[depth];
                    let leaf = o.kids == 0;
                    let one_leaf = o.kids == 1 && o.k0_leaf;
                    let g3a = (leaf && i == o.at as usize + 1) || one_leaf;
                    let g3b = leaf || one_leaf;
                    let g2a = o.kids <= 2 && o.all_g3a;
                    let g2b = o.kids <= 3 && o.all_g3b;
                    let g1 = match o.kids {
                        0 => true,
                        1 => o.k0_g2a || o.k0_g2b,
                        2 => o.k0_g2a && o.k1_g2b,
                        _ => false,
                    };
                    if p.n < p.g.len() {
                        p.g[p.n] = (o.at, i as u32, g1);
                        p.n += 1;
                    }
                    if depth > 0 && depth - 1 < stack.len() {
                        let parent = &mut stack[depth - 1];
                        parent.kids = parent.kids.saturating_add(1);
                        if parent.kids == 1 {
                            parent.k0_leaf = leaf;
                            parent.k0_g2a = g2a;
                            parent.k0_g2b = g2b;
                        } else if parent.kids == 2 {
                            parent.k1_g2b = g2b;
                        }
                        parent.all_g3a &= g3a;
                        parent.all_g3b &= g3b;
                    }
                }
            }
            i += 1;
        }
        p.g[..p.n].sort_unstable_by_key(|g| g.0);
        p
    }

    /// Where the group opened at `open` closes, when it is one `just_eat` takes.
    fn fitting(&self, open: usize) -> Option<usize> {
        let i = self.g[..self.n]
            .binary_search_by_key(&(open as u32), |g| g.0)
            .ok()?;
        let (_, close, fits) = self.g[i];
        fits.then_some(close as usize)
    }
}

// ---- the frames

#[derive(Clone, Copy)]
struct Frame {
    id: u8,
    /// Inside a make substitution's `variable.parameter` ([`TWIN_MK`]).
    param: bool,
    /// The kind of the text in it no token takes.
    base: u16,
}

/// The frames a scan holds.
const MAX: usize = 128;

struct Scan<'a, 'o, 'p> {
    b: &'a [u8],
    n: usize,
    /// The frames below `depth` are written; those above are not.
    stack: [MaybeUninit<Frame>; MAX],
    depth: usize,
    /// Where the spans so far end.
    emitted: usize,
    out: &'o mut Out<'p>,
    /// The `\` a line continuation is at (`\` and spaces to the line's end), or `usize::MAX`.
    cont: usize,
    parens: Option<Parens>,
    /// `var_ahead`'s and `rule_ahead`'s last answers, from where: still the answer from a later
    /// place up to it (none stays none), so a line of many substitutions in `main` reads them once.
    var_memo: (usize, Option<usize>),
    rule_memo: (usize, Option<usize>),
    /// The shell's state across lines, the topmost shell region's: its frame's depth (the
    /// frame below it is the region's; 0 for none), and bash's state.
    sh: (usize, State),
}

/// How a shell region's text on a line ends.
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    /// At the line's end.
    Eol,
    /// A make token goes on past the line's end, its frames open above the region's.
    Open,
    /// At a byte that ends the region (`)`, an embed's quote), the byte the frame below's.
    At(usize),
    /// At a `)` that ends `!=`'s, taken with it.
    Past(usize),
}

/// A shell region's make tokens on a line, in order: where each starts and ends, and the byte
/// it stands as in the shell's text (0 for none).
struct Cuts {
    n: usize,
    at: [(u32, u32, u8); 16],
    more: Vec<(u32, u32, u8)>,
}

impl Cuts {
    fn new() -> Self {
        Cuts {
            n: 0,
            at: [(0, 0, 0); 16],
            more: Vec::new(),
        }
    }

    fn push(&mut self, s: usize, e: usize) {
        let c = (s as u32, e as u32, 0);
        if self.n < self.at.len() {
            self.at[self.n] = c;
        } else {
            self.more.push(c);
        }
        self.n += 1;
    }

    fn entry(&mut self, i: usize) -> &mut (u32, u32, u8) {
        if i < self.at.len() {
            &mut self.at[i]
        } else {
            &mut self.more[i - self.at.len()]
        }
    }

    fn get(&self, i: usize) -> (usize, usize) {
        let (s, e, _) = if i < self.at.len() {
            self.at[i]
        } else {
            self.more[i - self.at.len()]
        };
        (s as usize, e as usize)
    }

    /// The byte token `i` stands as in the shell's text, 0 for none.
    fn stand(&self, i: usize) -> u8 {
        if i < self.at.len() {
            self.at[i].2
        } else {
            self.more[i - self.at.len()].2
        }
    }

    /// Each token's byte in the shell's text of `b`'s region from `p` to `end`: a run of
    /// adjacent tokens stands as one byte, the last's (`$` for a lone `$$`, which make hands the
    /// shell as `$`; else `_`, a word), but none where the run starts a word and the word goes on
    /// after it (`$(Q)echo`), or the next word is an assignment (`$(ENV) CC=gcc`, the run an
    /// environment more likely than a command): bash reads those as bat's shell does, the tokens
    /// unseen.
    fn stand_in(&mut self, b: &[u8], p: usize, end: usize) {
        let mut i = 0;
        while i < self.n {
            let mut j = i;
            while j + 1 < self.n && self.get(j + 1).0 == self.get(j).1 {
                j += 1;
            }
            let (s, e) = (self.get(i).0, self.get(j).1);
            let starts = s == p
                || matches!(
                    b[s - 1],
                    b' ' | b'\t' | b';' | b'|' | b'&' | b'(' | b'<' | b'>' | b'`'
                );
            let goes_on = e < end
                && !matches!(
                    b[e],
                    b' ' | b'\t' | b';' | b'|' | b'&' | b'(' | b')' | b'<' | b'>'
                );
            // `NAME=` or `NAME+=` after the run's blanks
            let assigns = || {
                let mut k = e;
                while k < end && matches!(b[k], b' ' | b'\t') {
                    k += 1;
                }
                let w = k;
                while k < end && (b[k].is_ascii_alphanumeric() || b[k] == b'_') {
                    k += 1;
                }
                k > w
                    && !b[w].is_ascii_digit()
                    && (b[k..end].starts_with(b"=") || b[k..end].starts_with(b"+="))
            };
            self.entry(j).2 = if i == j && &b[s..e] == b"$$" {
                b'$'
            } else if starts && (goes_on || assigns()) {
                0
            } else {
                b'_'
            };
            i = j + 1;
        }
    }
}

/// Lex `b`, a line of a Makefile without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let n = b.len();
    // the line continuation: a `\` then spaces to the line's end
    let mut k = n;
    while k > 0 && b[k - 1] == b' ' {
        k -= 1;
    }
    let cont = if k > 0 && b[k - 1] == b'\\' {
        k - 1
    } else {
        usize::MAX
    };
    let mut s = Scan {
        b,
        n,
        stack: [const { MaybeUninit::uninit() }; MAX],
        depth: 1,
        emitted: 0,
        out,
        cont,
        parens: None,
        var_memo: (usize::MAX, None),
        rule_memo: (usize::MAX, None),
        sh: (0, State::default()),
    };
    s.stack[0] = MaybeUninit::new(Frame {
        id: MAIN,
        param: false,
        base: PLAIN,
    });
    s.load(state);
    if let Some(p) = s.start() {
        s.run(p, 0);
    }
    s.fill(n);
    *state = s.save();
}

/// What a step did: `Ok` with where the line goes on from, `Err` where the line ends with the
/// frames as they are.
type Step = Result<usize, ()>;

impl Scan<'_, '_, '_> {
    // ---- the line

    /// The byte at `i`: past the line's end a `\n` (syntect's line ends in one), then 0.
    fn at(&self, i: usize) -> u8 {
        match self.b.get(i) {
            Some(&c) => c,
            None if i == self.n => b'\n',
            None => 0,
        }
    }

    /// Whether the character at `i` is a word character (`\w`).
    fn word_at(&self, i: usize) -> bool {
        match self.b.get(i) {
            None => false,
            Some(&c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
            Some(_) => self.char_at(i).is_some_and(char::is_alphanumeric),
        }
    }

    /// Whether the character before `i` is a word character.
    fn word_before(&self, i: usize) -> bool {
        match i.checked_sub(1).map(|j| self.b[j]) {
            None => false,
            Some(c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
            Some(_) => self.char_before(i).is_some_and(char::is_alphanumeric),
        }
    }

    #[cold]
    #[inline(never)]
    fn char_at(&self, i: usize) -> Option<char> {
        std::str::from_utf8(&self.b[i..(i + 4).min(self.n)])
            .or_else(|e| std::str::from_utf8(&self.b[i..i + e.valid_up_to()]))
            .ok()
            .and_then(|t| t.chars().next())
    }

    #[cold]
    #[inline(never)]
    fn char_before(&self, i: usize) -> Option<char> {
        let mut s = i - 1;
        while s > 0 && i - s < 4 && self.b[s] & 0xc0 == 0x80 {
            s -= 1;
        }
        std::str::from_utf8(&self.b[s..i])
            .ok()
            .and_then(|t| t.chars().next_back())
    }

    /// The end of the blanks (`\s`) from `i`.
    fn ws_end(&self, mut i: usize) -> usize {
        while i < self.n && ws(self.b[i]) {
            i += 1;
        }
        i
    }

    /// Where the blanks (`\s`) before `i` start, not before `lo`.
    fn ws_back(&self, lo: usize, mut i: usize) -> usize {
        while i > lo && ws(self.b[i - 1]) {
            i -= 1;
        }
        i
    }

    /// The first `x` at or after `from`.
    fn find(&self, from: usize, x: u8) -> Option<usize> {
        let i = find_in(self.b, from.min(self.n), self.n, [x]);
        (i < self.n).then_some(i)
    }

    /// The first `word` starting from `from` to `to` (inclusive).
    fn find_str_to(&self, from: usize, to: usize, word: &[u8]) -> Option<usize> {
        let mut at = from;
        while let Some(i) = self.find_to(at, to, word[0]) {
            if self.b[i..].starts_with(word) {
                return Some(i);
            }
            at = i + 1;
        }
        None
    }

    /// The first stop of `mask` at or after `p`, or the line's end.
    #[inline(always)]
    fn stop(&self, mut p: usize, mask: u32) -> usize {
        while p < self.n && CLS[usize::from(self.b[p])] & mask == 0 {
            p += 1;
        }
        p
    }

    // ---- the stack

    fn frame(&self, i: usize) -> &Frame {
        debug_assert!(i < self.depth);
        // SAFETY: every frame below `depth` was written (`push`, and the bottom one in `line`);
        // `depth` never passes `MAX`
        unsafe { self.stack.get_unchecked(i).assume_init_ref() }
    }

    fn top(&self) -> &Frame {
        self.frame(self.depth - 1)
    }

    /// The frame below the top's base.
    fn below_base(&self) -> u16 {
        self.frame(self.depth.saturating_sub(2)).base
    }

    fn push(&mut self, id: u8) {
        let below = *self.top();
        let own = BASE[usize::from(id)];
        let f = Frame {
            id,
            param: below.param || matches!(id, SUBST_P | SUBST_B),
            base: if own != PLAIN { own } else { below.base },
        };
        if self.depth == MAX {
            // the bottom frame above `main` dropped, and the shell region's depth with it
            self.stack.copy_within(2.., 1);
            self.depth -= 1;
            self.sh.0 = match self.sh.0 {
                0..=2 => 0,
                d => d - 1,
            };
        }
        self.stack[self.depth] = MaybeUninit::new(f);
        self.depth += 1;
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
    }

    /// The top frame replaced by `id` (syntect's `set`).
    fn set(&mut self, id: u8) {
        self.pop();
        self.push(id);
    }

    // ---- spans

    fn put(&mut self, s: usize, e: usize, kind: u16) {
        let e = e.min(self.n);
        if e > s && kind != PLAIN {
            let kind = if self.top().param && super::is_variable(kind) {
                kind | TWIN_MK
            } else {
                kind
            };
            self.out.push_merged(s, e, kind);
        }
        if e > self.emitted {
            self.emitted = e;
        }
    }

    /// The text up to `to` no token took, in the top frame's kind.
    fn fill(&mut self, to: usize) {
        if to > self.emitted {
            let base = self.top().base;
            self.put(self.emitted, to, base);
        }
    }

    /// A token `s..e` in `kind`, the text before it filled.
    fn tok(&mut self, s: usize, e: usize, kind: u16) {
        self.fill(s);
        self.put(s, e, kind);
    }

    /// The line's continuation at `i`: its `\`, the spaces after it in the top frame's kind,
    /// and the next line's blanks to come.
    fn continued(&mut self, i: usize) -> Step {
        self.tok(i, i + 1, mk::CONTINUATION);
        self.fill(self.n);
        self.push(CONT);
        Err(())
    }

    // ---- the line's start

    /// The line's start, by the frames left open: `None` when the line is done.
    fn start(&mut self) -> Option<usize> {
        let b = self.b;
        let n = self.n;
        loop {
            let id = self.top().id;
            match id {
                CONT => {
                    // `(?=\S|^\s*$)`: the blanks the construct's, then the construct
                    let i = self.ws_end(0);
                    if i < n {
                        self.fill(i);
                        self.pop();
                        return Some(i);
                    }
                    self.pop();
                    if self.top().id == MAIN {
                        return Some(0);
                    }
                }
                DEFINE_VALUE | DEFINE_NESTED => {
                    let i = self.ws_end(0);
                    let word = |w: &[u8]| b[i..].starts_with(w) && !self.word_at(i + w.len());
                    if word(b"endef") {
                        let e = i + 5;
                        if id == DEFINE_VALUE {
                            let below = self.below_base();
                            self.put(0, i, below);
                            self.put(i, e, mk::CONTROL);
                        } else {
                            self.fill(e);
                        }
                        self.pop();
                        return Some(e);
                    }
                    if id == DEFINE_VALUE && word(b"define") {
                        self.push(DEFINE_NESTED);
                    }
                    self.fill(n);
                    return None;
                }
                JUNCTION => {
                    let i = self.ws_end(0);
                    if self.at(i) == b'#' {
                        self.tok(i, i + 1, mk::COMMENT_MARK);
                        self.push(COMMENT);
                        return Some(i + 1);
                    }
                    if let Some(e) = self.if_directive(i) {
                        self.tok(i, e, mk::CONDITIONAL);
                        self.push(R_IF);
                        return Some(e);
                    }
                    match self.at(0) {
                        b' ' => self.set(R_SPACES),
                        b'\t' => self.set(R_TABS),
                        _ => self.pop(),
                    }
                }
                R_TABS | R_SPACES => {
                    let (own, other) = if id == R_TABS {
                        (b'\t', b' ')
                    } else {
                        (b' ', b'\t')
                    };
                    let c = self.at(0);
                    if c == own {
                        // the recipe's indent, its prefix, then the shell
                        let s = if own == b'\t' {
                            1
                        } else {
                            b.iter().position(|&c| c != b' ').unwrap_or(n)
                        };
                        let e = self.prefix_end(s);
                        self.tok(s, e, mk::CONST_LANG);
                        return Some(self.shell_start(e));
                    }
                    if c == other {
                        // the other indent: invalid, its prefix, the line plain
                        let s = if other == b'\t' {
                            1
                        } else {
                            b.iter().position(|&c| c != b' ').unwrap_or(n)
                        };
                        let e = self.prefix_end(s);
                        let bad = if id == R_TABS {
                            mk::EXPECTED_TAB
                        } else {
                            mk::EXPECTED_SPACES
                        };
                        self.put(0, s, bad);
                        self.put(s, e, mk::CONST_LANG);
                        return Some(e);
                    }
                    if n == 0 {
                        // `^\n`: a blank line keeps the recipe
                        return None;
                    }
                    if let Some(r) = self.control_flow(0) {
                        return r.ok();
                    }
                    self.pop();
                }
                _ => return Some(0),
            }
        }
    }

    /// `([-@]{1,2})?` from `i`: where it ends.
    fn prefix_end(&self, i: usize) -> usize {
        let mut e = i;
        while e < i + 2 && matches!(self.at(e), b'-' | b'@') {
            e += 1;
        }
        e
    }

    /// A recipe's shell from `p`: a comment right there (`\s*#`), or its commands.
    fn shell_start(&mut self, p: usize) -> usize {
        if let Some(c) = self.find(p, b'#')
            && self.ws_back(p, c) == p
        {
            self.put(p, c, super::k::COMMENT);
            self.put(c, c + 1, super::k::COMMENT_BEGIN);
            self.push(SH_COMMENT);
            return c + 1;
        }
        self.push(SH_RECIPE);
        p
    }

    /// `ifn?(def|eq)` at `i`: where it ends.
    fn if_directive(&self, i: usize) -> Option<usize> {
        let b = self.b;
        if !b[i.min(self.n)..].starts_with(b"if") {
            return None;
        }
        let tail = |j: usize| {
            if b[j.min(self.n)..].starts_with(b"def") {
                Some(j + 3)
            } else if b[j.min(self.n)..].starts_with(b"eq") {
                Some(j + 2)
            } else {
                None
            }
        };
        let j = i + 2;
        (self.at(j) == b'n')
            .then(|| tail(j + 1))
            .flatten()
            .or_else(|| tail(j))
    }

    /// `\bifn?(def|eq)\b` from `p`: where it starts and ends.
    fn ifdef(&self, p: usize) -> Option<(usize, usize)> {
        self.ifdef_to(p, self.n)
    }

    /// `\bifn?(def|eq)\b` starting from `p` to `to` (inclusive).
    fn ifdef_to(&self, p: usize, to: usize) -> Option<(usize, usize)> {
        let mut at = p;
        while let Some(i) = self.find_str_to(at, to, b"if") {
            if !self.word_before(i) {
                let j = i + 2;
                let ends = [
                    (self.at(j) == b'n').then(|| self.tail(j + 1)).flatten(),
                    self.tail(j),
                ];
                if let Some(e) = ends.into_iter().flatten().find(|&e| !self.word_at(e)) {
                    return Some((i, e));
                }
            }
            at = i + 1;
        }
        None
    }

    fn tail(&self, j: usize) -> Option<usize> {
        let r = &self.b[j.min(self.n)..];
        if r.starts_with(b"def") {
            Some(j + 3)
        } else if r.starts_with(b"eq") {
            Some(j + 2)
        } else {
            None
        }
    }

    /// `^\s*(KW)\b`: where `kw` starts and ends, at the line's start.
    fn bol_word(&self, kw: &[u8]) -> Option<(usize, usize)> {
        let i = self.ws_end(0);
        (self.b[i..].starts_with(kw) && !self.word_at(i + kw.len())).then(|| (i, i + kw.len()))
    }

    /// bat's `control-flow` at the line's start (`vpath`, `include`, `ifeq` and its kin,
    /// `else`, `endif`), where one is there; `ifeq` only where nothing comes before it.
    fn control_flow(&mut self, p: usize) -> Option<Step> {
        debug_assert_eq!(p, 0);
        let i = self.ws_end(0);
        if let Some(e) = self.vpath_at(i) {
            return Some(Ok(self.vpath(i, e)));
        }
        if let Some((s, e)) = self.include_at(i) {
            return Some(Ok(self.include(s, e)));
        }
        if i == 0
            && let Some(e) = self.if_directive(0)
            && !self.word_at(e)
        {
            self.put(0, e, mk::CONDITIONAL);
            self.push(COND);
            return Some(Ok(e));
        }
        if let Some((s, e)) = self.bol_word(b"else") {
            self.put(s, e, mk::CONDITIONAL);
            self.push(ELSE);
            return Some(Ok(e));
        }
        if let Some((s, e)) = self.bol_word(b"endif") {
            self.put(s, e, mk::CONDITIONAL);
            self.push(ENDIF);
            return Some(Ok(e));
        }
        None
    }

    /// `([s-]?include)\s+` at `i`: where the word starts and ends.
    fn include_at(&self, i: usize) -> Option<(usize, usize)> {
        let b = self.b;
        let j = if matches!(self.at(i), b's' | b'-')
            && b[(i + 1).min(self.n)..].starts_with(b"include")
        {
            i + 1
        } else {
            i
        };
        if !b[j.min(self.n)..].starts_with(b"include") {
            return None;
        }
        let e = j + 7;
        ws(self.at(e)).then_some((i, e))
    }

    /// `^\s*([s-]?include)\s+` matched, the word at `s..e`: the keyword, its blanks, the paths.
    fn include(&mut self, s: usize, e: usize) -> usize {
        let end = self.ws_end(e);
        self.tok(s, e, mk::IMPORT);
        let base = self.top().base;
        self.put(e, end, base);
        self.push(INCLUDE);
        end
    }

    /// `vpath\s` at `i` (the `\s` the line's `\n` too): where the word ends.
    fn vpath_at(&self, i: usize) -> Option<usize> {
        let e = i + 5;
        (self.b[i.min(self.n)..].starts_with(b"vpath") && ws(self.at(e))).then_some(e)
    }

    /// `^\s*(vpath)\s` matched to `e`: the keyword, then its pattern (none past the line's end,
    /// where the `\s` was its `\n`).
    fn vpath(&mut self, i: usize, e: usize) -> usize {
        self.tok(i, e, mk::VPATH);
        if e < self.n {
            let base = self.top().base;
            self.put(e, e + 1, base);
            self.push(VPATH);
            return e + 1;
        }
        e
    }

    // ---- `just_eat`'s lookaheads

    fn parens(&mut self) -> &Parens {
        if self.parens.is_none() {
            self.parens = Some(Parens::new(self.b));
        }
        self.parens.as_ref().unwrap()
    }

    /// The next paren at or after `q`, or the line's end.
    fn next_paren(&self, q: usize) -> usize {
        find_in(self.b, q.min(self.n), self.n, *b"()")
    }

    /// The last `:` not followed by `=` in `a..r`.
    fn last_colon(&self, a: usize, r: usize) -> Option<usize> {
        let mut r = r;
        while let Some(c) = self.b[a..r].iter().rposition(|&x| x == b':') {
            let c = a + c;
            if self.at(c + 1) != b'=' {
                return Some(c);
            }
            r = c;
        }
        None
    }

    fn has_eq(&self, a: usize, r: usize) -> bool {
        a < r && find_in(self.b, a, r, *b"=") < r
    }

    /// The group opened at `r`, when `just_eat` takes it: where it closes, and where the text
    /// after it runs to.
    fn group_at(&mut self, r: usize) -> Option<(usize, usize)> {
        if self.at(r) != b'(' {
            return None;
        }
        let close = self.parens().fitting(r)?;
        Some((close, self.next_paren(close + 1)))
    }

    /// `(?={{rule_lookahead}})`: `just_eat` then a `:` that isn't `:=`. From `q` the colon is in
    /// the run of text up to the next paren, or in the run after the group that paren opens.
    fn rule_ahead(&mut self, pos: usize) -> Option<usize> {
        let (from, memo) = self.rule_memo;
        if from <= pos && memo.is_none_or(|q| q >= pos) {
            return memo;
        }
        let r = self.rule_search(pos);
        self.rule_memo = (pos, r);
        r
    }

    fn rule_search(&mut self, pos: usize) -> Option<usize> {
        let mut q = pos;
        while q < self.n {
            let r = self.next_paren(q);
            let group = self.group_at(r);
            if self.last_colon(q, r).is_some()
                || group.is_some_and(|(c, r1)| self.last_colon(c + 1, r1).is_some())
            {
                return Some(q);
            }
            q = r + 1;
        }
        None
    }

    /// `(?={{var_lookahead}}|{{first_assign_then_colon}})`: an `=` after `just_eat` where no rule
    /// colon is.
    fn var_ahead(&mut self, pos: usize) -> Option<usize> {
        let (from, memo) = self.var_memo;
        if from <= pos && memo.is_none_or(|v| v >= pos) {
            return memo;
        }
        let r = self.var_search(pos);
        self.var_memo = (pos, r);
        r
    }

    fn var_search(&mut self, pos: usize) -> Option<usize> {
        let mut q = pos;
        while q < self.n {
            let r = self.next_paren(q);
            let group = self.group_at(r);
            let colon_after = group.is_some_and(|(c, r1)| self.last_colon(c + 1, r1).is_some());
            let eq_after = group.is_some_and(|(c, r1)| self.has_eq(c + 1, r1));
            if !colon_after {
                let v = self.last_colon(q, r).map_or(q, |c| c + 1);
                if v < r && (self.has_eq(v, r) || eq_after) {
                    return Some(v);
                }
                if v == r && self.at(r) == b'(' && eq_after {
                    return Some(r);
                }
            }
            q = r + 1;
        }
        None
    }

    // ---- the run

    /// The line from `p` until the frames are back to `floor` (`Some`, where) or the line ends.
    fn run(&mut self, mut p: usize, floor: usize) -> Option<usize> {
        loop {
            if self.depth <= floor {
                return Some(p);
            }
            let id = self.top().id;
            let step = if id >= SH_RECIPE {
                self.shell(p, id)
            } else {
                self.step(p, id)
            };
            match step {
                Ok(e) => p = e,
                Err(()) => return None,
            }
        }
    }

    /// A frame's step from `p`.
    fn step(&mut self, p: usize, id: u8) -> Step {
        match id {
            MAIN => return self.classify(p),
            TVAR => return self.defs(p, false),
            OVERRIDE => return self.defs(p, true),
            CONT => {
                let i = self.ws_end(p);
                if i < self.n {
                    self.fill(i);
                    self.pop();
                    return Ok(i);
                }
                return Err(());
            }
            COMMENT | SH_COMMENT => {
                if self.cont != usize::MAX && self.cont >= p {
                    return self.continued(self.cont);
                }
                self.fill(self.n);
                self.pop();
                return Ok(self.n);
            }
            RULE_COMMENT => {
                self.fill(self.n);
                self.set(JUNCTION);
                return Err(());
            }
            INLINE => {
                if p >= self.n {
                    self.set(JUNCTION);
                    return Err(());
                }
                if self.b[p] == b'#' {
                    self.tok(p, p + 1, mk::COMMENT_MARK);
                    self.push(COMMENT);
                    return Ok(p + 1);
                }
                return Ok(self.shell_start(p));
            }
            JUNCTION => {
                if let Some(h) = self.find(p, b'#') {
                    self.tok(h, h + 1, mk::COMMENT_MARK);
                    self.push(COMMENT);
                    return Ok(h + 1);
                }
                self.fill(self.n);
                return Err(());
            }
            R_TABS | R_SPACES => {
                if let Some((s, e)) = self.ifdef(p) {
                    self.tok(s, e, mk::CONDITIONAL);
                    self.push(COND);
                    return Ok(e);
                }
                self.fill(self.n);
                return Err(());
            }
            CALL_SHELL => {
                if self.at(p) == b')' {
                    self.tok(p, p + 1, mk::BLOCK_END);
                    self.pop();
                    return Ok(p + 1);
                }
                self.push(SH_CALL);
                return Ok(p);
            }
            MAYBE_SHELL => {
                if let Some(e) = self.shell_c(p) {
                    return Ok(e);
                }
                self.set(VALUE);
                return Ok(p);
            }
            DEFINE_VALUE | DEFINE_NESTED | VPATH_DIRS => {
                self.fill(self.n);
                if id == VPATH_DIRS {
                    self.pop();
                    return Ok(self.n);
                }
                return Err(());
            }
            DEFINE => return self.define(p),
            _ => {}
        }
        let i = self.stop(p, STOP[usize::from(id)]);
        if i >= self.n {
            return self.eol(id);
        }
        let c = self.b[i];
        match (id, c) {
            (_, b'$') => Ok(self.subst(i).unwrap_or(i + 1)),
            (_, b'\\') => {
                // an escape where the frame has them, else the line's continuation
                let escapes = matches!(id, VALUE | STR_S | STR_D);
                if escapes && i + 1 < self.n {
                    let e = i + 1 + char_len(self.b[i + 1]);
                    self.tok(i, e, mk::ESCAPE);
                    return Ok(e);
                }
                if matches!(id, FCALL | CALL_PAT | CALL_WILD) {
                    // `\\['"]`: the quote not a string's
                    if matches!(self.at(i + 1), b'\'' | b'"') {
                        return Ok(i + 2);
                    }
                    return Ok(i + 1);
                }
                if i == self.cont {
                    return self.continued(i);
                }
                Ok(i + 1)
            }
            (_, b'#') => {
                self.tok(i, i + 1, mk::COMMENT_MARK);
                match id {
                    VALUE => self.set(COMMENT),
                    PREREQ => self.set(RULE_COMMENT),
                    _ => self.push(COMMENT),
                }
                Ok(i + 1)
            }
            (_, b'%') => {
                self.tok(i, i + 1, mk::PERCENT);
                Ok(i + 1)
            }
            (_, b'*') if id == VPATH || id == CALL_WILD => {
                self.tok(i, i + 1, mk::WILDCARD);
                Ok(i + 1)
            }
            (COND | R_IF, _) => {
                match c {
                    b'\'' => {
                        self.tok(i, i + 1, mk::STR_BEGIN);
                        self.push(STR_S);
                    }
                    b'"' => {
                        self.tok(i, i + 1, mk::STR_BEGIN);
                        self.push(STR_D);
                    }
                    b'(' => {
                        self.tok(i, i + 1, mk::GROUP_BEGIN);
                        self.push(GROUP);
                    }
                    _ => self.tok(i, i + 1, mk::STRAY_PAREN),
                }
                Ok(i + 1)
            }
            (STR_S, b'\'') | (STR_D, b'"') => {
                self.tok(i, i + 1, mk::STR_END);
                self.pop();
                Ok(i + 1)
            }
            (GROUP, b')') => {
                self.tok(i, i + 1, mk::GROUP_END);
                self.pop();
                Ok(i + 1)
            }
            (GROUP | FCALL | CALL_PAT | CALL_WILD | CALL_INFO, b',') => {
                self.tok(i, i + 1, mk::SEPARATOR);
                Ok(i + 1)
            }
            (VPATH, _) => {
                // `\s`: the directories
                self.fill(i);
                self.put(i, i + 1, self.top().base);
                self.set(VPATH_DIRS);
                Ok(i + 1)
            }
            (ELSE, _) => {
                if let Some((s, e)) = self.ifdef_to(i, i) {
                    self.tok(s, e, mk::CONDITIONAL);
                    self.push(COND);
                    return Ok(e);
                }
                Ok(i + 1)
            }
            (TARGET, b':') => Ok(self.rule_colon(i)),
            (PREREQ, b';') => {
                // `(?=\s*;)`: the blanks before it the rule's, then the recipe
                let q = self.ws_back(self.emitted, i);
                self.fill(q);
                let below = self.below_base();
                self.put(q, i, below);
                self.put(i, i + 1, mk::TERMINATOR);
                self.set(INLINE);
                Ok(i + 1)
            }
            (FCALL | CALL_PAT | CALL_WILD | CALL_INFO | SUBST_P, b')') | (SUBST_B, b'}') => {
                self.tok(i, i + 1, mk::BLOCK_END);
                self.pop();
                Ok(i + 1)
            }
            (FCALL | CALL_PAT | CALL_WILD | CALL_INFO, b'(') => {
                self.fill(i);
                self.push(BAL);
                Ok(i + 1)
            }
            (FCALL | CALL_PAT | CALL_WILD, _) => {
                // a quote: `['"](?=\s*,)` passed, else a string
                if self.at(self.ws_end(i + 1)) == b',' {
                    return Ok(i + 1);
                }
                self.tok(i, i + 1, mk::STR_BEGIN);
                self.push(if c == b'\'' { STR_S } else { STR_D });
                Ok(i + 1)
            }
            (CALL, _) => {
                // `(?=\))` or `,`: the name ends, the arguments follow
                self.fill(i);
                if c == b',' {
                    self.set(FCALL);
                    self.put(i, i + 1, mk::SEPARATOR);
                    return Ok(i + 1);
                }
                self.set(FCALL);
                Ok(i)
            }
            (BAL, _) => {
                self.fill(i + 1);
                self.pop();
                Ok(i + 1)
            }
            (SUBST_P | SUBST_B, b':') => {
                self.tok(i, i + 1, mk::SUBST_MARK);
                Ok(i + 1)
            }
            (SUBST_P | SUBST_B, _) => {
                self.tok(i, i + 1, mk::ASSIGN_MARK);
                Ok(i + 1)
            }
            (EXPORT, _) => {
                // `(\?|\+|::?)?=`: the value
                let s = self.op_start::<false>(self.emitted, i);
                self.tok(s, i + 1, mk::ASSIGN);
                self.pop();
                Ok(self.value(i + 1, false))
            }
            (VARIABLE, _) => Ok(self.assign(i)),
            (EMBED_SQ | EMBED_DQ | EMBED_BT, _) => {
                self.tok(i, i + 1, mk::INTERP_END);
                self.pop();
                Ok(i + 1)
            }
            _ => Ok(i + 1),
        }
    }

    /// A frame at the line's end.
    fn eol(&mut self, id: u8) -> Step {
        self.fill(self.n);
        match id {
            COND | R_IF | INCLUDE | ELSE | ENDIF | EXPORT | VALUE => {
                self.pop();
                Ok(self.n)
            }
            PREREQ => {
                self.set(JUNCTION);
                Err(())
            }
            VPATH => {
                // `\s` takes the line's `\n`, and the directories end with it
                self.pop();
                Ok(self.n)
            }
            _ => Err(()),
        }
    }

    /// `main`'s patterns from `p`, the leftmost, ties to the grammar's first: a comment, the
    /// definitions (`override`, `define`, `export`, an assignment), a rule, a substitution, the
    /// directives. The assignment and the rule are found first, the commonest lines; the others
    /// only where they'd start no later.
    fn classify(&mut self, p: usize) -> Step {
        const HASH: u8 = 0;
        const OVERRIDE_KW: u8 = 1;
        const DEFINE_KW: u8 = 2;
        const EXPORT_KW: u8 = 3;
        const VAR: u8 = 4;
        const RULE: u8 = 5;
        const SUBST: u8 = 6;
        const VPATH_KW: u8 = 7;
        const INCLUDE_KW: u8 = 9;
        const IFDEF: u8 = 10;
        const ELSE_KW: u8 = 11;
        const ENDIF_KW: u8 = 12;
        const ENDEF_KW: u8 = 13;
        let n = self.n;
        if p >= n {
            return Err(());
        }
        let i = self.ws_end(p);
        if p == 0 && self.at(i) == b'#' {
            self.tok(i, i + 1, mk::COMMENT_MARK);
            self.push(COMMENT);
            return Ok(i + 1);
        }
        let var = self.var_ahead(p);
        // a rule can't win over an assignment at the start
        let rule = if var == Some(p) {
            None
        } else {
            self.rule_ahead(p)
        };
        let mut win = (usize::MAX, u8::MAX);
        let take = |s: Option<usize>, rank: u8, win: &mut (usize, u8)| {
            if let Some(s) = s
                && (s, rank) < *win
            {
                *win = (s, rank);
            }
        };
        take(var, VAR, &mut win);
        take(rule, RULE, &mut win);
        // the nearest substitution bounds the searches of those ranked before it
        take(self.first_subst(p, win.0), SUBST, &mut win);
        take(self.find_to(p, win.0, b'#'), HASH, &mut win);
        let ov = self.word_ws(p, win.0, b"override");
        take(ov.map(|w| w.0), OVERRIDE_KW, &mut win);
        let df = self.word_ws(p, win.0, b"define");
        take(df.map(|w| w.0), DEFINE_KW, &mut win);
        let export = if p == 0 {
            self.bol_word(b"export")
        } else {
            None
        };
        take(export.map(|_| 0), EXPORT_KW, &mut win);
        let at0 = p == 0 && win.0 > 0;
        let vp = if at0 { self.vpath_at(i) } else { None };
        take(vp.map(|_| 0), VPATH_KW, &mut win);
        let inc = if at0 { self.include_at(i) } else { None };
        take(inc.map(|_| 0), INCLUDE_KW, &mut win);
        let ifd = if win.0 > p {
            self.ifdef_to(p, win.0 - 1)
        } else {
            None
        };
        take(ifd.map(|w| w.0), IFDEF, &mut win);
        let els = if at0 { self.bol_word(b"else") } else { None };
        take(els.map(|_| 0), ELSE_KW, &mut win);
        let endif = if at0 { self.bol_word(b"endif") } else { None };
        take(endif.map(|_| 0), ENDIF_KW, &mut win);
        let endef = at0 && self.b[i..].starts_with(b"endef");
        take(endef.then_some(0), ENDEF_KW, &mut win);
        let (s, rank) = win;
        match rank {
            HASH => {
                self.tok(s, s + 1, mk::COMMENT_MARK);
                self.push(COMMENT);
                Ok(s + 1)
            }
            OVERRIDE_KW => Ok(self.keyword(s, ov.unwrap().1, 8, OVERRIDE)),
            DEFINE_KW => Ok(self.keyword(s, df.unwrap().1, 6, DEFINE)),
            EXPORT_KW => {
                let (k, e) = export.unwrap();
                self.tok(k, e, mk::CONTROL);
                self.push(EXPORT);
                Ok(e)
            }
            VAR => {
                self.fill(s);
                self.push(VARIABLE);
                Ok(s)
            }
            RULE => {
                self.fill(s);
                self.push(TARGET);
                Ok(s)
            }
            SUBST => {
                self.fill(s);
                Ok(self.subst(s).unwrap_or(s + 1))
            }
            VPATH_KW => Ok(self.vpath(i, vp.unwrap())),
            INCLUDE_KW => {
                let (k, e) = inc.unwrap();
                Ok(self.include(k, e))
            }
            IFDEF => {
                let (k, e) = ifd.unwrap();
                self.tok(k, e, mk::CONDITIONAL);
                self.push(COND);
                Ok(e)
            }
            ELSE_KW | ENDIF_KW => {
                let (k, e) = if rank == ELSE_KW { els } else { endif }.unwrap();
                self.tok(k, e, mk::CONDITIONAL);
                self.push(if rank == ELSE_KW { ELSE } else { ENDIF });
                Ok(e)
            }
            ENDEF_KW => {
                self.tok(i, i + 5, mk::STRAY_ENDEF);
                Ok(i + 5)
            }
            _ => {
                self.fill(n);
                Err(())
            }
        }
    }

    /// The first `x` from `from` to `to` (inclusive).
    fn find_to(&self, from: usize, to: usize, x: u8) -> Option<usize> {
        let end = to.saturating_add(1).min(self.n);
        let i = find_in(self.b, from.min(end), end, [x]);
        (i < end).then_some(i)
    }

    /// `\s*(WORD)\b` from `p`, starting no later than `best` (a tie the word's): where the
    /// match and the word start.
    fn word_ws(&self, p: usize, best: usize, word: &[u8]) -> Option<(usize, usize)> {
        let to = if best >= self.n {
            self.n
        } else {
            self.ws_end(best)
        };
        let mut at = p;
        while let Some(k) = self.find_str_to(at, to, word) {
            let s = self.ws_back(p, k);
            if s > best {
                return None;
            }
            if !self.word_at(k + word.len()) {
                return Some((s, k));
            }
            at = k + 1;
        }
        None
    }

    /// `\s*(KW)\b` matched `s..k+len`: the keyword, its frame pushed.
    fn keyword(&mut self, s: usize, k: usize, len: usize, id: u8) -> usize {
        self.fill(s);
        let base = self.top().base;
        self.put(s, k, base);
        self.put(k, k + len, mk::CONTROL);
        if id == OVERRIDE && self.top().id != MAIN {
            self.set(id);
        } else {
            self.push(id);
        }
        k + len
    }

    /// The definitions after a target's `:` (`TVAR`) or `override` (`define_word`): the leftmost
    /// of `define` (as a word, after `override`), `override`, `define`, an assignment, a
    /// substitution (not after `override`), the line's continuation, its end.
    fn defs(&mut self, p: usize, define_word: bool) -> Step {
        let n = self.n;
        let var = self.var_ahead(p);
        let mut win = (usize::MAX, u8::MAX);
        let take = |s: Option<usize>, rank: u8, win: &mut (usize, u8)| {
            if let Some(s) = s
                && (s, rank) < *win
            {
                *win = (s, rank);
            }
        };
        take(var, 4, &mut win);
        if !define_word {
            take(self.first_subst(p, win.0), 5, &mut win);
        }
        // `override` first: the nearest, it bounds the search for `define` before it
        let ov = self.word_ws(p, win.0, b"override");
        take(ov.map(|w| w.0), 1, &mut win);
        let mut dw = None;
        if define_word {
            let mut at = p;
            while let Some(k) = self.find_str_to(at, win.0, b"define") {
                if !self.word_before(k) && !self.word_at(k + 6) {
                    dw = Some(k);
                    break;
                }
                at = k + 1;
            }
        }
        take(dw, 0, &mut win);
        let df = self.word_ws(p, win.0, b"define");
        take(df.map(|w| w.0), 2, &mut win);
        if self.cont != usize::MAX && self.cont >= p {
            take(Some(self.cont), 6, &mut win);
        }
        let (s, rank) = win;
        match rank {
            0 => Ok(self.keyword(s, s, 6, DEFINE)),
            1 => Ok(self.keyword(s, ov.unwrap().1, 8, OVERRIDE)),
            2 => Ok(self.keyword(s, df.unwrap().1, 6, DEFINE)),
            4 => {
                self.fill(s);
                self.push(VARIABLE);
                Ok(s)
            }
            5 => {
                self.fill(s);
                Ok(self.subst(s).unwrap_or(s + 1))
            }
            6 => self.continued(s),
            _ => {
                self.fill(n);
                self.pop();
                Ok(n)
            }
        }
    }

    /// The first `$` from `p` to `to` (inclusive) a substitution starts at.
    fn first_subst(&self, p: usize, to: usize) -> Option<usize> {
        let mut at = p;
        while let Some(i) = self.find_to(at, to, b'$') {
            if self.subst_end(i).is_some() {
                return Some(i);
            }
            at = i + 1;
        }
        None
    }

    /// A rule's `:` at `c`: a target-specific variable's (an `=` after it, and it isn't `:=`), or
    /// the rule's, its prerequisites after it.
    fn rule_colon(&mut self, c: usize) -> usize {
        // `(?= *::?)`: the spaces before it the rule's
        let mut q = c;
        while q > self.emitted && self.b[q - 1] == b' ' {
            q -= 1;
        }
        self.fill(q);
        let below = self.below_base();
        self.put(q, c, below);
        let e = if self.at(c + 1) == b':' { c + 2 } else { c + 1 };
        let last_eq = self.b.iter().rposition(|&x| x == b'=');
        if self.at(c + 1) != b'=' && last_eq.is_some_and(|l| l > c + 1) {
            self.put(c, e, mk::ASSIGN);
            self.set(TVAR);
            return e;
        }
        self.put(c, e, mk::ASSIGN);
        let end = self.ws_end(e);
        self.put(e, end, below);
        self.set(PREREQ);
        end
    }

    /// Where `(\?|\+|::?)?=` (with `!` when `BANG`) ending at the `=` at `f` starts, not before
    /// `pos`.
    fn op_start<const BANG: bool>(&self, pos: usize, f: usize) -> usize {
        let b = self.b;
        if f >= pos + 2 && b[f - 2] == b':' && b[f - 1] == b':' {
            f - 2
        } else if f > pos && (matches!(b[f - 1], b'?' | b'+' | b':') || (BANG && b[f - 1] == b'!'))
        {
            f - 1
        } else {
            f
        }
    }

    /// A variable's `=` at `f`: its operator, then the value (or `!=`'s shell).
    fn assign(&mut self, f: usize) -> usize {
        let s = self.op_start::<true>(self.emitted, f);
        let q = self.ws_back(self.emitted, s);
        self.fill(q);
        self.pop();
        let base = self.top().base;
        self.put(q, s, base);
        self.put(s, f + 1, mk::ASSIGN);
        if self.b[s] == b'!' {
            self.push(SH_ASSIGN);
            return f + 1;
        }
        self.value(f + 1, true)
    }

    /// A value from `p`, past its leading blanks: `bash -c '…'` (where `shell`), else text.
    fn value(&mut self, p: usize, shell: bool) -> usize {
        let e = self.ws_end(p);
        self.put(p, e, PLAIN);
        if shell {
            self.push(MAYBE_SHELL);
        } else {
            self.push(VALUE);
        }
        e
    }

    /// `((?:bash|sh|zsh)\s+-c\s+)(['"`])` at `p`: the shell's word unquoted, the quote an
    /// interpolation's, the embedded script's frames.
    fn shell_c(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let r = &b[p.min(self.n)..];
        let w = if r.starts_with(b"bash") {
            4
        } else if r.starts_with(b"zsh") {
            3
        } else if r.starts_with(b"sh") {
            2
        } else {
            return None;
        };
        let k = self.ws_end(p + w);
        if k == p + w || !b[k.min(self.n)..].starts_with(b"-c") {
            return None;
        }
        let q = self.ws_end(k + 2);
        if q == k + 2 || q >= self.n {
            return None;
        }
        let (embed, region) = match b[q] {
            b'\'' => (EMBED_SQ, SH_SQ),
            b'"' => (EMBED_DQ, SH_DQ),
            b'`' => (EMBED_BT, SH_BT),
            _ => return None,
        };
        self.put(p, q, mk::UNQUOTED);
        self.put(q, q + 1, mk::INTERP_BEGIN);
        self.push(embed);
        self.push(region);
        Some(q + 1)
    }

    /// A `define` line from `p`: the name, then an operator ending the line (`=`, `:=` and the
    /// like), then the body.
    fn define(&mut self, p: usize) -> Step {
        let n = self.n;
        // the line's last character but blanks, when it is an `=`
        let t = (0..n).rev().find(|&i| !ws(self.b[i]));
        let op = t
            .filter(|&t| t >= p && self.b[t] == b'=')
            .map(|t| (self.op_start::<true>(p, t), t + 1));
        let to = op.map_or(n, |(s, _)| s);
        let mut at = p;
        while let Some(i) = self.find(at, b'$').filter(|&i| i < to) {
            if let Some(e) = self.subst(i) {
                if self.top().id != DEFINE {
                    return Ok(e);
                }
                at = e;
            } else {
                at = i + 1;
            }
        }
        self.fill(to);
        if let Some((s, e)) = op {
            self.pop();
            self.put(s, e, mk::ASSIGN);
            self.fill(n);
            self.push(DEFINE_VALUE);
            return Err(());
        }
        self.set(DEFINE_VALUE);
        Err(())
    }

    // ---- substitutions

    /// The substitution at the `$` at `i`, as [`Scan::subst`] would take it: where it ends.
    fn subst_end(&self, i: usize) -> Option<usize> {
        let c = self.at(i + 1);
        let (o, after) = if c == b'$' {
            (i + 2, self.at(i + 2))
        } else {
            (i + 1, c)
        };
        if after == b'(' || after == b'{' {
            return Some(o + 1);
        }
        if matches!(after, b'@' | b'%' | b'<' | b'?' | b'^' | b'+' | b'|' | b'*') {
            return Some(o + 1);
        }
        if c == b'$' {
            return Some(i + 2);
        }
        if i + 1 < self.n && self.b[i + 1].is_ascii_alphabetic() {
            return Some(i + 2);
        }
        if i + 1 < self.n && self.b[i + 1] >= 0x80 {
            return self
                .char_at(i + 1)
                .filter(|c| c.is_alphabetic())
                .map(|c| i + 1 + c.len_utf8());
        }
        None
    }

    /// `variable-substitutions` at the `$` at `i`: a function's call, `$(…)`, `${…}`, an
    /// automatic variable, `$$`, `$x`; painted, its frame pushed; where it ends.
    fn subst(&mut self, i: usize) -> Option<usize> {
        let c = self.at(i + 1);
        let (o, after) = if c == b'$' {
            (i + 2, self.at(i + 2))
        } else {
            (i + 1, c)
        };
        if after == b'(' {
            // `(\$\$?\()(NAME)\s`: the name's word read once, then looked up
            let name = o + 1;
            let mut e = name;
            while e < self.n && (self.b[e].is_ascii_lowercase() || self.b[e] == b'-') {
                e += 1;
            }
            let word = &self.b[name.min(self.n)..e.max(name.min(self.n))];
            if let Some(&(_, id)) = (e > name && ws(self.at(e)))
                .then(|| CALLS.iter().find(|c| c.0 == word))
                .flatten()
            {
                self.fill(i);
                self.put(i, name, mk::BLOCK_BEGIN);
                self.put(
                    name,
                    e,
                    if id == CALL {
                        mk::CALL_LANG
                    } else {
                        mk::BUILTIN
                    },
                );
                let base = self.top().base;
                self.put(e, e + 1, base);
                self.push(id);
                return Some((e + 1).min(self.n));
            }
        }
        // `\$\$?\(`, `\$\$?{`, `\$\$?[@%<?^+|*]`, then `\$\$`, `(\$)[[:alpha:]]`
        let e = self.subst_end(i)?;
        self.fill(i);
        match after {
            b'(' | b'{' if e == o + 1 => {
                self.put(i, e, mk::BLOCK_BEGIN);
                self.push(if after == b'(' { SUBST_P } else { SUBST_B });
            }
            b'@' | b'%' | b'<' | b'?' | b'^' | b'+' | b'|' | b'*' if e == o + 1 => {
                self.put(i, e, mk::AUTOMATIC);
            }
            _ if c == b'$' => self.put(i, e, mk::ESCAPE),
            _ => {
                self.put(i, i + 1, mk::SINGLE_CHAR);
                self.put(i + 1, e, mk::VAR_PARAM);
            }
        }
        Some(e)
    }

    // ---- shell regions

    /// The shell region on top, from `p`: its make tokens run on make's frames, each a byte in
    /// the shell's text or none ([`Cuts::stand_in`]), which bash's scanner lexes from the state
    /// the region kept; the region popped where it ends on the line, else kept with bash's state
    /// (the module's doc).
    #[inline(never)]
    fn shell(&mut self, p: usize, id: u8) -> Step {
        let b = self.b;
        let n = self.n;
        let d = self.depth;
        let mut st = if self.sh.0 == d {
            self.sh.0 = 0;
            std::mem::take(&mut self.sh.1)
        } else {
            State::default()
        };
        if p >= n {
            // nothing of the region on the line: a recipe's and `!=`'s end, the others go on
            if matches!(id, SH_RECIPE | SH_ASSIGN) {
                self.pop();
                return Ok(n);
            }
            self.keep_shell(d, st);
            return Err(());
        }
        // the make tokens, and where the region's text on the line ends; the tokens' text that
        // no make scope takes is bash's at the token's byte, so the region's kind is out of it
        let frame = *self.top();
        let region = frame.base;
        self.stack[d - 1] = MaybeUninit::new(Frame {
            base: PLAIN,
            ..frame
        });
        let bit = 1u8 << (id - SH_RECIPE);
        let mut cuts = Cuts::new();
        let mark = self.out.0.len();
        let mut q = p;
        let (end, mut stop) = loop {
            let Some(i) = b[q..]
                .iter()
                .position(|&c| TRIGGER[usize::from(c)] & bit != 0)
            else {
                break (n, Stop::Eol);
            };
            let t = q + i;
            let opened = match b[t] {
                b'\\' => {
                    // bash's escape: the byte after it is the shell's
                    q = if t + 1 < n {
                        t + 1 + char_len(b[t + 1])
                    } else {
                        n
                    };
                    continue;
                }
                b')' if id == SH_ASSIGN => break (t, Stop::Past(t)),
                b')' => break (t, Stop::At(t)),
                b'\'' | b'"' | b'`' if id != SH_CALL => break (t, Stop::At(t)),
                b'$' => {
                    self.emitted = t;
                    self.subst(t)
                }
                c => {
                    self.emitted = t;
                    match c {
                        b'\'' => {
                            self.put(t, t + 1, mk::STR_BEGIN);
                            self.push(STR_S);
                            Some(t + 1)
                        }
                        b'"' => {
                            self.put(t, t + 1, mk::STR_BEGIN);
                            self.push(STR_D);
                            Some(t + 1)
                        }
                        _ => {
                            self.push(BAL);
                            Some(t + 1)
                        }
                    }
                }
            };
            let Some(e) = opened else {
                q = t + 1;
                continue;
            };
            match self.run(e, d) {
                Some(u) => {
                    cuts.push(t, u);
                    q = u.min(n);
                }
                None => {
                    // its text to the line's end is the open frames'
                    self.fill(n);
                    cuts.push(t, n);
                    break (n, Stop::Open);
                }
            }
        };
        self.stack[d - 1] = MaybeUninit::new(frame);
        // the shell's text: the region's on the line, a token a byte in it, as make hands the
        // shell a word where one stands, and `$` for `$$` ([`Cuts::stand_in`])
        cuts.stand_in(b, p, end);
        let mut small = [0u8; 256];
        let heap: Vec<u8>;
        // under a token open past the line's end, bash's contexts stay open, as bat's stay under
        // make's frames: the shell's text ends in a `\` of its own, mapped to nothing
        let open = usize::from(stop == Stop::Open);
        let view: &[u8] = if cuts.n == 0 {
            &b[p..end]
        } else {
            let mut len = end - p + open;
            for i in 0..cuts.n {
                let (s, e) = cuts.get(i);
                len = len - (e - s) + usize::from(cuts.stand(i) != 0);
            }
            let mut v = Vec::new();
            let buf: &mut [u8] = if len <= small.len() {
                &mut small[..len]
            } else {
                v.resize(len, 0);
                &mut v
            };
            let mut w = 0;
            let mut from = p;
            for i in 0..=cuts.n {
                let to = if i < cuts.n { cuts.get(i).0 } else { end };
                buf[w..w + to - from].copy_from_slice(&b[from..to]);
                w += to - from;
                if i < cuts.n {
                    let c = cuts.stand(i);
                    if c != 0 {
                        buf[w] = c;
                        w += 1;
                    }
                    from = cuts.get(i).1;
                }
            }
            if open == 1 {
                buf[w] = b'\\';
            }
            heap = v;
            if len <= small.len() {
                &small[..len]
            } else {
                &heap
            }
        };
        let own_at = self.out.0.len();
        super::line_spans(view, &mut st, self.out);
        let own_end = self.out.0.len();
        // a recipe's comment, as bash reads it, takes the line's rest: make's tokens after its
        // `#` were the comment's text (bat's comment from the prototype has none of make's
        // patterns)
        let mut comment = usize::MAX;
        if id == SH_RECIPE
            && let Some(at) = self.out.0[own_at..own_end]
                .iter()
                .find(|sp| sp.kind == super::k::COMMENT_BEGIN)
                .map(|sp| sp.start as usize)
        {
            let mut v = at;
            let mut o = p;
            for i in 0..cuts.n {
                let (s, e) = cuts.get(i);
                if o + v < s {
                    break;
                }
                v = v.saturating_sub(s - o + usize::from(cuts.stand(i) != 0));
                o = e;
            }
            comment = o + v;
            if stop == Stop::Open && cuts.get(cuts.n - 1).0 >= comment {
                // the open token was the comment's: its frames go
                self.depth = d;
            }
            if stop == Stop::Open {
                stop = Stop::Eol;
            }
        }
        self.lay(p, end, region, &cuts, (mark, own_at, own_end), comment);
        self.emitted = end;
        match stop {
            Stop::At(t) => {
                self.pop();
                Ok(t)
            }
            Stop::Past(t) => {
                let below = self.below_base();
                self.pop();
                self.put(t, t + 1, below);
                Ok(t + 1)
            }
            Stop::Open => {
                if self.sh.0 == 0 {
                    self.sh = (d, st);
                }
                Err(())
            }
            Stop::Eol => {
                // a recipe goes on past a `\` ending it, as bash's `\\\n` keeps bat's shell;
                // `!=`'s ends with its line; `$(shell …)` and an embed go on
                let goes_on = match id {
                    SH_RECIPE => {
                        comment == usize::MAX
                            && (cuts.n == 0 || cuts.get(cuts.n - 1).1 < n)
                            && b.iter().rev().take_while(|&&c| c == b'\\').count() % 2 == 1
                    }
                    SH_ASSIGN => false,
                    _ => true,
                };
                if goes_on {
                    self.keep_shell(d, st);
                    Err(())
                } else {
                    self.pop();
                    Ok(n)
                }
            }
        }
    }

    /// bash's state kept for the region at depth `d`, where no region above it keeps one: a
    /// state holds the topmost region's, and one below it starts afresh once it's back on top.
    fn keep_shell(&mut self, d: usize, st: State) {
        if self.sh.0 <= d {
            self.sh = (d, st);
        }
    }

    /// The region's spans on the line laid in order at `mark`: bash's (at `own.1..own.2`, on the
    /// shell's text) mapped back around the tokens' bytes, the gaps in the region's kind
    /// (`region`), and each token's spans (at `own.0..own.1`), its gaps in bash's kind at its
    /// byte, a variable inside an option twinned; past `comment`, the tokens are the comment's
    /// text.
    fn lay(
        &mut self,
        p: usize,
        end: usize,
        region: u16,
        cuts: &Cuts,
        (mark, own_at, own_end): (usize, usize, usize),
        comment: usize,
    ) {
        let top = *self.top();
        let v = &mut *self.out.0;
        let start = v.len();
        let emit = |v: &mut Vec<crate::Span>, s: usize, e: usize, kind: u16| {
            if e <= s || kind == PLAIN {
                return;
            }
            if v.len() > start
                && let Some(l) = v.last_mut()
                && l.end as usize == s
                && l.kind == kind
            {
                l.end = e as u32;
                return;
            }
            v.push(crate::Span {
                start: s as u32,
                end: e as u32,
                kind,
            });
        };
        let own_kind = |kind: u16| {
            if top.param && super::is_variable(kind & !super::TWIN) {
                (kind & !super::TWIN) | TWIN_MK
            } else {
                kind
            }
        };
        let (mut ti, mut oi) = (mark, own_at);
        let mut vo = 0;
        // where the run of tokens' bytes the current one ends started, in the shell's text
        let mut run = 0;
        for k in 0..=cuts.n {
            let ps = if k == 0 { p } else { cuts.get(k - 1).1 };
            let pe = if k < cuts.n { cuts.get(k).0 } else { end };
            let len = pe - ps;
            let mut at = ps;
            while oi < own_end {
                let sp = v[oi];
                let (s, e) = (sp.start as usize, sp.end as usize);
                if s >= vo + len {
                    break;
                }
                let (cs, ce) = (s.max(vo), e.min(vo + len));
                if ce > cs {
                    emit(v, at, ps + cs - vo, region);
                    emit(v, ps + cs - vo, ps + ce - vo, own_kind(sp.kind));
                    at = ps + ce - vo;
                }
                if e <= vo + len {
                    oi += 1;
                } else {
                    break;
                }
            }
            emit(v, at, pe, region);
            if k == 0 || len > 0 {
                run = vo + len;
            }
            vo += len;
            if k == cuts.n {
                break;
            }
            let stands = cuts.stand(k) != 0;
            let kind_at = if !stands {
                // a token bash didn't see: bash's kind through it, where a span runs on past it
                match v.get(oi) {
                    Some(sp)
                        if oi < own_end && (sp.start as usize) < vo && sp.end as usize > vo =>
                    {
                        sp.kind
                    }
                    _ => PLAIN,
                }
            } else {
                // the token's byte: bash's kind there, and past it; a command's name that starts
                // at it, or at the tokens' bytes right before it, isn't one yet where the token
                // stands (bash's text before it opened nothing), so the token's text is the
                // region's
                let mut kind_at = PLAIN;
                while oi < own_end {
                    let sp = v[oi];
                    if sp.start as usize > vo {
                        break;
                    }
                    if sp.end as usize > vo
                        && !(sp.start as usize >= run
                            && matches!(sp.kind, super::COMMAND | super::k::FUNCTION_CALL))
                    {
                        kind_at = sp.kind;
                    }
                    if sp.end as usize <= vo + 1 {
                        oi += 1;
                    } else {
                        break;
                    }
                }
                vo += 1;
                kind_at
            };
            let (cs, ce) = cuts.get(k);
            let twin = stands && in_option(kind_at);
            let base = if kind_at == PLAIN {
                region
            } else {
                own_kind(kind_at)
            };
            // the token's spans, each cut to it: make's writes join adjacent tokens' spans of a
            // kind (a run of `$$`)
            let mut at = cs;
            while ti < own_at && (v[ti].start as usize) < ce {
                let sp = v[ti];
                let (s, e) = ((sp.start as usize).max(cs), (sp.end as usize).min(ce));
                if cs < comment && e > s {
                    let kind = if twin
                        && sp.kind & (TWIN_MK | super::TWIN) == 0
                        && super::is_variable(sp.kind)
                    {
                        sp.kind | super::TWIN
                    } else {
                        sp.kind
                    };
                    emit(v, at, s, base);
                    emit(v, s, e, kind);
                    at = e;
                }
                if sp.end as usize <= ce {
                    ti += 1;
                } else {
                    break;
                }
            }
            if cs >= comment {
                emit(v, cs, ce, super::k::COMMENT);
            } else {
                emit(v, at, ce.min(end), base);
            }
        }
        v.drain(mark..start);
    }

    // ---- the state

    /// The frames `state` holds, above the bottom one, and the shell's state above them.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        let held = state.held();
        let frames = usize::from(state.flags() as u8).min(held);
        for i in 0..frames {
            let id = state.level(i);
            if usize::from(id) >= FRAMES {
                break;
            }
            self.push(id);
        }
        if held > frames {
            // the topmost shell region's
            let mut st = State {
                depth: (held - frames) as u8,
                ..State::default()
            };
            for i in frames..held {
                st.set_level(i - frames, state.level(i));
            }
            if let Some(d) = (1..self.depth)
                .rev()
                .find(|&d| self.frame(d).id >= SH_RECIPE)
            {
                self.sh = (d + 1, st);
            }
        }
    }

    /// The frames from the top, as many as a state holds, and the shell's state above them
    /// where it fits.
    #[inline(always)]
    fn save(&self) -> State {
        let from = self.depth - (self.depth - 1).min(HELD);
        let frames = self.depth - from;
        let mut state = State {
            depth: frames as u8,
            arg: frames as u16,
            ..State::default()
        };
        for d in from..self.depth {
            state.set_level(d - from, self.frame(d).id);
        }
        let (d, st) = &self.sh;
        if *d != 0 && *d <= self.depth && self.frame(*d - 1).id >= SH_RECIPE {
            let held = st.held();
            if frames + held <= HELD {
                for i in 0..held {
                    state.set_level(frames + i, st.level(i));
                }
                state.depth += held as u8;
            }
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    /// A command's name, in bash's color for it ([`super::super::COMMAND`]).
    fn command() -> String {
        Lang::Makefile.scopes(super::super::COMMAND).join(" ")
    }

    #[test]
    fn a_rule_and_its_recipe() {
        let command = command();
        let m = marked(
            Lang::Makefile,
            &[
                "CC := gcc",
                "all: main.o $(OBJS)",
                "\techo \"$(CC)\" | grep -q gcc # check",
            ],
        );
        assert!(m.contains(&("CC", "variable.other.makefile")));
        assert!(m.contains(&(
            "all",
            "meta.function.makefile entity.name.function.makefile"
        )));
        assert!(m.contains(&("OBJS", "variable.parameter.makefile")));
        // the recipe is bash, with make's `$(…)` inside its string
        assert!(m.contains(&(
            "echo",
            "meta.function-call.shell support.function.echo.shell"
        )));
        assert!(m.contains(&("CC", "variable.parameter.makefile")));
        assert!(m.contains(&("|", "keyword.operator.logical.pipe.shell")));
        assert!(m.contains(&("grep", command.as_str())));
        assert!(m.contains(&(" check", "comment.line.number-sign.shell")));
    }

    #[test]
    fn shell_assignments_and_calls() {
        let command = command();
        let m = marked(
            Lang::Makefile,
            &[
                "DATE != date +%F",
                "REV := $(shell git rev-parse HEAD)",
                "X = $(subst a,b,$(Y))",
            ],
        );
        assert!(m.contains(&("!=", "keyword.operator.assignment.makefile")));
        assert!(m.contains(&("date", command.as_str())));
        assert!(m.contains(&("shell", "support.function.builtin.makefile")));
        assert!(m.contains(&("git", command.as_str())));
        assert!(m.contains(&("subst", "support.function.builtin.makefile")));
        assert!(m.contains(&(",", "punctuation.separator.makefile")));
    }

    #[test]
    fn a_define_spans_lines() {
        let m = marked(
            Lang::Makefile,
            &["define RULE", "$(1): ; echo hi", "endef", "all: x"],
        );
        assert!(m.contains(&("define", "keyword.control.makefile")));
        assert!(m.contains(&("endef", "keyword.control.makefile")));
        assert!(m.contains(&(
            "all",
            "meta.function.makefile entity.name.function.makefile"
        )));
    }

    /// A make token is a word in the shell's text, and `$$` its `$` (X-094): a flag after a
    /// token is an option, `$$f` a shell variable; but a token starting a word that goes on is
    /// unseen (`$(Q)echo`), and so is one before an assignment (`$(ENV) CC=gcc`).
    #[test]
    fn make_tokens_in_the_shell() {
        let m = marked(
            Lang::Makefile,
            &[
                "x: y",
                "\t$(CC) -c $< -o$@",
                "\tfor f in $^; do echo $$f; done",
                "\t$(Q)echo hi",
                "\t$(ENV) CC=gcc make",
                // a run of tokens whose spans make joins, each laid as its own (`tokens`' 281
                // findings at X-094: they overlapped)
                "\techo \"$$$$$$$$$$$6.2KiB$$$$\"",
            ],
        );
        assert!(m.contains(&("c", "variable.parameter.option.shell")));
        assert!(m.contains(&("$<", "variable.language.automatic.makefile")));
        // inside `-o`: the option's scope outranks the automatic variable's
        assert!(m.contains(&(
            "$@",
            "variable.parameter.option.shell variable.language.automatic.makefile"
        )));
        assert!(m.contains(&("$$", "constant.character.escape.makefile")));
        assert!(m.contains(&("f", "variable.other.readwrite.shell")));
        assert!(m.contains(&(
            "echo",
            "meta.function-call.shell support.function.echo.shell"
        )));
        assert!(m.contains(&("CC", "variable.other.readwrite.assignment.shell")));
    }

    /// A token open past a recipe's line keeps the recipe under it, and make's lines after;
    /// bash's contexts stay open under one (`x=$$(a \`'s value).
    #[test]
    fn an_open_token_across_lines() {
        let m = marked(
            Lang::Makefile,
            &[
                "ifeq (a,b)",
                "x: y",
                "\t$(eval A += b \\",
                "\t\tc)",
                "endif",
                "V := $(shell x=$$(a \\",
                "\tb)c && echo $$x)",
                "ifeq (a,b)",
                "endif",
            ],
        );
        let endifs = m
            .iter()
            .filter(|t| **t == ("endif", "keyword.control.conditional.makefile"))
            .count();
        assert_eq!(endifs, 2, "{m:?}");
        assert!(m.contains(&("c", "string.unquoted.shell")), "{m:?}");
    }

    /// No limit of syntect's on nesting (X-133): past its 99 levels a substitution and a
    /// function call still open.
    #[test]
    fn nesting_past_syntects_depth() {
        let line = format!("x := {}$(info y)", "$(".repeat(120));
        let m = marked(Lang::Makefile, &[&line]);
        assert!(
            m.contains(&("info", "support.function.builtin.makefile")),
            "{m:?}"
        );
    }
}
