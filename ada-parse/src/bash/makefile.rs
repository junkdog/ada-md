//! Makefiles (ADR-40): make's own contexts on a frame stack of their own, and the shell in
//! recipes, `!=`, `$(shell …)` and `bash -c '…'` on bash's scanner ([`super::line_spans`], X-094).
//! bat's `Makefile` is the reference for make's scopes and the parity oracle.
//!
//! The Makefile grammar's own contexts hold few patterns, most of them searches (`just_eat`'s
//! lookahead, `\s*(override)\b`, a `$(` anywhere), so they keep syntect's rule, the leftmost
//! match of the context's patterns in order, each context's includes flattened at compile time
//! ([`FLAT`]). Each pattern is a matcher of `matchers.rs`, its match kept for the line as
//! syntect's search cache keeps it ([`Mk`]), so a line costs a search a pattern rather than one a
//! step; and a matcher whose required byte the line lacks isn't called ([`REQUIRE`]).
//! `variable-substitutions`' eleven patterns, all starting at a `$`, are one search
//! ([`substitution`]).
//!
//! **A shell region is a frame** ([`SH_RECIPE`] and the others), where bat sets
//! `scope:source.shell` with make's patterns in front (a `with_prototype`, or an embed's
//! escape). Its text up to where the region ends (a line's end, the `)` closing `$(shell …)`, an
//! embed's quote) is read for make's tokens first: `$(…)`, `${…}`, `$$`, `$@` and the like
//! everywhere, and in `$(shell …)` make's strings and parentheses too. Each runs on make's
//! frames above the region's, as bat runs them above the shell's contexts. bash's scanner then
//! lexes the region's text in one call, from the state the region kept, **reading it as make
//! hands it to the shell**: a token stands as a word (`_`), `$$` as `$` ([`Cuts::stand_in`]),
//! and its spans are laid back around the tokens', a token's text that no make scope takes in
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
//! Between lines the state holds make's frames, a byte each, and above them the shell's state
//! where a region goes on past the line (a recipe's `\`, an open `$(shell …)`).

use std::mem::MaybeUninit;

use super::matchers as g;
use super::scan::Line;
use super::scan::{self, M, R};
use crate::prim::char_len;
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

// ---- contexts: bat's, by its names

macro_rules! contexts {
    ($($c:ident,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u8)]
        enum CtxId { $($c,)* Count }
        $(const $c: u8 = CtxId::$c as u8;)*
        /// How many contexts the Makefile grammar has.
        const COUNT: usize = CtxId::Count as usize;
    };
}

contexts! {
    MAIN,
    INSIDE_CONTROL_FLOW,
    STRING_SINGLE,
    STRING_DOUBLE,
    CONTROL_GROUP,
    VPATH,
    VPATH_DIRS,
    INCLUDE,
    ELSE,
    ENDIF,
    EXPECT_RULE,
    TARGET_VARIABLE,
    TARGET_VARIABLE_VALUE,
    RULE_COLON,
    PREREQUISITES,
    RULE_COMMENT_START,
    RULE_COMMENT,
    RULE_SEMICOLON,
    RECIPE_JUNCTION,
    RECIPE_IF,
    RECIPE_INLINE,
    RECIPE_WITH_SPACES,
    RECIPE_WITH_TABS,
    SHELL,
    SHELL_COMMENT,
    MK_CONTINUED,
    COMMENT_START,
    MK_COMMENT,
    INSIDE_FUNCTION_CALL,
    CALL,
    CALL_COMMA,
    CALL_PATSUBST,
    CALL_WILDCARD,
    CALL_INFO,
    CALL_SHELL,
    OVERRIDE,
    EXPORT,
    VARIABLE,
    SHELL_ASSIGN,
    ASSIGN,
    PAREN_BALANCER,
    EAT_WHITESPACE,
    VALUE_MAYBE_SHELL,
    EMBEDDED_SCRIPT_SQ,
    EMBEDDED_SCRIPT_DQ,
    EMBEDDED_SCRIPT_BT,
    VALUE,
    VALUE_COMMENT_START,
    DEFINE,
    DEFINE_ASSIGN,
    DEFINE_VALUE,
    DEFINE_NESTED,
    SUBSTITUTION_PAREN,
    SUBSTITUTION_BRACE,
}

// ---- shell regions: where bat sets `scope:source.shell` with make's patterns in front

/// A recipe's commands, to the line's end (or on past a `\` ending it).
const SH_RECIPE: u8 = COUNT as u8;
/// `$(shell …)`: to the `)` that closes it, make's strings and parentheses its own.
const SH_CALL: u8 = SH_RECIPE + 1;
/// `VAR != …`: to the line's end, or a `)`, taken.
const SH_ASSIGN: u8 = SH_RECIPE + 2;
/// `bash -c '…'`, `"…"`, `` `…` ``: to the quote.
const SH_SQ: u8 = SH_RECIPE + 3;
const SH_DQ: u8 = SH_RECIPE + 4;
const SH_BT: u8 = SH_RECIPE + 5;
/// Contexts with the shell regions.
const CONTEXTS: usize = COUNT + 6;

/// Per context, its meta scope's kind and its meta content scope's (a non-meta leaf, or
/// [`PLAIN`]).
static INFO: [(u16, u16); CONTEXTS] = {
    let mut t = [(PLAIN, PLAIN); CONTEXTS];
    macro_rules! set {
        ($($c:ident: $meta:expr, $content:expr;)*) => {
            $(t[$c as usize] = ($meta, $content);)*
        };
    }
    set! {
        STRING_SINGLE: mk::STR_SINGLE, PLAIN;
        STRING_DOUBLE: mk::STR_DOUBLE, PLAIN;
        VPATH_DIRS: PLAIN, mk::UNQUOTED;
        INCLUDE: PLAIN, mk::UNQUOTED;
        EXPECT_RULE: mk::TARGET, PLAIN;
        PREREQUISITES: PLAIN, mk::PREREQ;
        RULE_COMMENT: mk::COMMENT, PLAIN;
        SHELL_COMMENT: super::k::COMMENT, PLAIN;
        MK_COMMENT: mk::COMMENT, PLAIN;
        CALL: PLAIN, mk::VAR_FUNCTION;
        EXPORT: PLAIN, mk::VAR_OTHER;
        VARIABLE: PLAIN, mk::VAR_OTHER;
        VALUE: PLAIN, mk::UNQUOTED;
        DEFINE: PLAIN, mk::VAR_OTHER;
        DEFINE_VALUE: PLAIN, mk::UNQUOTED;
        SUBSTITUTION_PAREN: mk::VAR_PARAM, PLAIN;
        SUBSTITUTION_BRACE: mk::VAR_PARAM, PLAIN;
    }
    t
};

/// Per byte, the shell regions a make token or the region's end may start on, a bit each
/// (`1 << (ctx - SH_RECIPE)`): a `$` in every region but an embed's, `\` (bash's escape, which
/// takes the byte after it from make, as bat's prototype does) where bash's prototype comes
/// before make's patterns, `)` ending `$(shell …)` and `!=`, make's strings and parentheses in
/// `$(shell …)`, an embed's quote.
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

// ---- matchers: the port's, each with its argument

type Find = fn(&Line, usize, u16) -> R;

macro_rules! matchers {
    ($($id:ident = $f:expr, $a:expr;)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u8)]
        enum Mid { $($id,)* Count }
        $(const $id: u8 = Mid::$id as u8;)*
        const MATCHERS: usize = Mid::Count as usize;
        static FIND: [(Find, u16); MATCHERS] = [$(($f, $a),)*];
    };
}

/// Per matcher, a byte every match of it holds (0 for none): where the line lacks it, the
/// matcher isn't called.
static REQUIRE: [u8; MATCHERS] = {
    let mut t = [0u8; MATCHERS];
    macro_rules! req {
        ($($c:literal: $($id:ident),*;)*) => {
            $($(t[$id as usize] = $c;)*)*
        };
    }
    req! {
        b'\'': X_SQ, X_QUOTE_SQ;
        b'"': X_DQ, X_QUOTE_DQ;
        b'`': X_QUOTE_BT;
        b'(': X_OPEN;
        b')': X_CLOSE, X_CLOSE_AHEAD;
        b',': X_COMMA, X_COMMA_AHEAD;
        b'%': X_PERCENT;
        b'*': X_STAR;
        b'#': X_HASH, X_HASH_AHEAD, X_COMMENT_AHEAD, X_HASH_WS;
        b';': X_SEMI, X_SEMI_AHEAD;
        b':': X_COLON, X_COLON_EQ_AHEAD, X_COLON_AHEAD, X_COLONS_WS, X_COLONS, X_RULE_AHEAD;
        b'=': X_EQ, X_VAR_AHEAD, X_ASSIGN_OP, X_ASSIGN_OP_BANG, X_ASSIGN_AHEAD,
            X_DEFINE_OP_AHEAD, X_DEFINE_ASSIGN;
        b'}': X_BRACE_CLOSE;
        b'!': X_SHELL_ASSIGN_AHEAD, X_BANG_EQ;
        b'\\': X_LINE_CONT, X_ESCAPE;
        b'$': X_SUBST;
        b'f': X_ENDEF_STRAY, X_ENDEF, X_DEFINE_NESTED, X_ENDIF, X_IFDEF, X_RECIPE_IF,
            X_DEFINE_WS, X_DEFINE_WORD;
        b'l': X_ELSE;
        b'x': X_EXPORT;
        b'v': X_VPATH_WS, X_VPATH_EOL, X_OVERRIDE;
        b'u': X_INCLUDE;
        b'h': X_SHELL_C;
    }
    t
};

/// Per byte, its bit in a line's [`Mk::present`] when a matcher requires it ([`REQUIRE`]),
/// else 0: the line's set is the bytes' bits or'ed, one load each.
static BIT: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut next = 0;
    let mut m = 0;
    while m < MATCHERS {
        let c = REQUIRE[m] as usize;
        if c != 0 && t[c] == 0 {
            t[c] = 1 << next;
            next += 1;
        }
        m += 1;
    }
    assert!(next <= 32, "a bit for each required byte");
    t
};

const _: () = assert!(MATCHERS <= 128, "a valid bit for each matcher");

/// `'`, `"` or `` ` ``: the quote an embedded script ends at.
fn quote<const Q: u8>(l: &Line, pos: usize, _: u16) -> R {
    g::byte::<Q>(l, pos, 0)
}

/// `((?:bash|sh|zsh)\s+-c\s+)(['"`])`: each of the words holds `sh`, so only where an `sh` is
/// can one start: two bytes before it (`bash`), one (`zsh`), or there.
fn shell_c(l: &Line, pos: usize, _: u16) -> R {
    let b = l.b;
    let mut at = pos;
    while let Some(j) = scan::find(b, at, b's') {
        at = j + 1;
        if l.at(j + 1) != b'h' {
            continue;
        }
        let starts = [
            (j >= pos + 2 && b[j - 2..j] == *b"ba").then(|| (j - 2, j + 2)),
            (j > pos && b[j - 1] == b'z').then(|| (j - 1, j + 2)),
            Some((j, j + 2)),
        ];
        for (i, word_end) in starts.into_iter().flatten() {
            let k = scan::ws_end(b, word_end);
            if k == word_end || !b[k..].starts_with(b"-c") {
                continue;
            }
            let q = scan::ws_end(b, k + 2);
            if q > k + 2 && matches!(l.at(q), b'\'' | b'"' | b'`') {
                return Some(M::at(i, q + 1).cap(1, i, q).cap(2, q, q + 1));
            }
        }
    }
    None
}

/// `variable-substitutions`, its eleven patterns in one search: each starts at a `$`, so the
/// leftmost match among them is at the first `$` where one matches, the first of them in the
/// grammar's order. Which one is group 5's start, an index into [`SUBSTITUTIONS`].
fn substitution(l: &Line, pos: usize, _: u16) -> R {
    let mut at = pos;
    while let Some(i) = scan::find(l.b, at, b'$') {
        if let Some((m, which)) = substitution_at(l, i) {
            return Some(m.cap(5, which, which));
        }
        at = i + 1;
    }
    None
}

/// The first of `variable-substitutions`' patterns that matches at the `$` at `i`.
fn substitution_at(l: &Line, i: usize) -> Option<(M, usize)> {
    let c = l.at(i + 1);
    // `\$\$?` then `(`, `{`, an automatic variable's character
    let (o, after) = if c == b'$' {
        (i + 2, l.at(i + 2))
    } else {
        (i + 1, c)
    };
    // `function-invocations`: `(\$\$?\()(NAME)\s`, by group
    if after == b'(' {
        let name = o + 1;
        let rest = &l.b[name.min(l.len())..];
        for (g, words) in g::CALL_NAMES.iter().enumerate() {
            for word in *words {
                let e = name + word.len();
                if rest.starts_with(word) && scan::ws(l.at(e)) {
                    return Some((M::at(i, e + 1).cap(1, i, name).cap(2, name, e), g));
                }
            }
        }
    }
    // `\$\$?\(`, `\$\$?{`, `\$\$?[@%<?^+|*]`: `$$` then the character, or `$` then it
    let pred = |x: u8| match x {
        b'(' => Some(6),
        b'{' => Some(7),
        b'@' | b'%' | b'<' | b'?' | b'^' | b'+' | b'|' | b'*' => Some(8),
        _ => None,
    };
    let dd = if c == b'$' { pred(l.at(i + 2)) } else { None };
    let d = pred(c);
    for which in 6..=8 {
        if dd == Some(which) {
            return Some((M::at(i, i + 3), which));
        }
        if d == Some(which) {
            return Some((M::at(i, i + 2), which));
        }
    }
    // `\$\$`
    if c == b'$' {
        return Some((M::at(i, i + 2), 9));
    }
    // `(\$)[[:alpha:]]`
    let n = l.alpha_at(i + 1)?;
    Some((M::at(i, i + 1 + n).cap(1, i, i + 1), 10))
}

matchers! {
    X_EMPTY = g::empty, 0;
    X_BOL = g::bol, 0;
    X_EOL = g::eol, 0;
    X_EOL_NL = g::eol_nl, 0;
    X_SQ = g::byte::<b'\''>, 0;
    X_DQ = g::byte::<b'"'>, 0;
    X_OPEN = g::byte::<b'('>, 0;
    X_CLOSE = g::byte::<b')'>, 0;
    X_COMMA = g::byte::<b','>, 0;
    X_PERCENT = g::byte::<b'%'>, 0;
    X_STAR = g::byte::<b'*'>, 0;
    X_HASH = g::byte::<b'#'>, 0;
    X_SEMI = g::byte::<b';'>, 0;
    X_COLON = g::byte::<b':'>, 0;
    X_EQ = g::byte::<b'='>, 0;
    X_BRACE_CLOSE = g::byte::<b'}'>, 0;
    X_CLOSE_AHEAD = g::ahead::<b')'>, 0;
    X_COMMA_AHEAD = g::ahead::<b','>, 0;
    X_HASH_AHEAD = g::ahead::<b'#'>, 0;
    X_WS1 = g::ws1, 0;
    X_NL = g::nl, 0;
    X_EAT_WS = g::eat_ws, 0;
    X_ENDEF_STRAY = g::endef_stray, 0;
    X_ENDEF = g::endef, 0;
    X_DEFINE_NESTED = g::define_nested, 0;
    X_ELSE = g::else_kw, 0;
    X_ENDIF = g::endif_kw, 0;
    X_EXPORT = g::export_kw, 0;
    X_VPATH_WS = g::vpath_ws, 0;
    X_VPATH_EOL = g::vpath_eol, 0;
    X_INCLUDE = g::include_kw, 0;
    X_IFDEF = g::ifdef, 0;
    X_RECIPE_IF = g::recipe_if, 0;
    X_COMMENT_AHEAD = g::comment_ahead, 0;
    X_LINE_CONT = g::line_cont, 0;
    X_CONT_END = g::cont_end, 0;
    X_ESCAPE = g::escape_any, 0;
    X_RULE_AHEAD = g::rule_ahead, 0;
    X_VAR_AHEAD = g::var_ahead, 0;
    X_COLON_EQ_AHEAD = g::colon_then_eq_ahead, 0;
    X_COLON_AHEAD = g::colon_ahead, 0;
    X_COLONS_WS = g::colons_ws, 0;
    X_COLONS = g::colons, 0;
    X_SEMI_AHEAD = g::semicolon_ahead, 0;
    X_SPACES_AHEAD = g::spaces_ahead, 0;
    X_TAB_AHEAD = g::tab_ahead, 0;
    X_RECIPE_SPACES = g::recipe_spaces, 0;
    X_RECIPE_TAB2 = g::recipe_tab::<false>, 0;
    X_RECIPE_TAB1 = g::recipe_tab::<true>, 0;
    X_NOT_SPACES = g::not_spaces, 0;
    X_NOT_TAB = g::not_tab, 0;
    X_BLANK = g::blank, 0;
    X_HASH_WS = g::hash_ws, 0;
    X_CALL_QUOTE = g::call_quote, 0;
    X_SUBST = substitution, 0;
    X_OVERRIDE = g::override_kw, 0;
    X_DEFINE_WS = g::define_ws, 0;
    X_DEFINE_WORD = g::define_word, 0;
    X_ASSIGN_OP = g::assign_op::<false>, 0;
    X_ASSIGN_OP_BANG = g::assign_op::<true>, 0;
    X_ASSIGN_AHEAD = g::assign_ahead, 0;
    X_SHELL_ASSIGN_AHEAD = g::shell_assign_ahead, 0;
    X_BANG_EQ = g::bang_eq, 0;
    X_SHELL_C = shell_c, 0;
    X_QUOTE_SQ = quote::<b'\''>, 0;
    X_QUOTE_DQ = quote::<b'"'>, 0;
    X_QUOTE_BT = quote::<b'`'>, 0;
    X_DEFINE_OP_AHEAD = g::define_op_ahead, 0;
    X_DEFINE_ASSIGN = g::define_assign, 0;
}

/// A Makefile's line as its make patterns read it, and their matches on it: each valid while
/// it starts at or after the place being matched, or when there is none from an earlier
/// place, as syntect's search cache keeps them.
pub struct Mk<'a> {
    l: Line<'a>,
    /// The required bytes the line holds, by their [`BIT`]s.
    present: u32,
    valid: [u64; 2],
    /// Where each match starts and ends, or [`NO_MATCH`].
    span: [MaybeUninit<(u32, u32)>; MATCHERS],
    found: [MaybeUninit<M>; MATCHERS],
}

/// A span's start where its matcher found none.
const NO_MATCH: u32 = u32::MAX;

// ---- patterns

/// A pattern: its matcher, the kind its match takes, its groups' kinds, what it does.
#[derive(Clone, Copy)]
struct Pat {
    m: u8,
    scope: u16,
    caps: &'static [(u8, u16)],
    op: Op<'static>,
    /// `bash -c '…'`: the contexts it pushes are the ones for its quote, group 2.
    quote: bool,
}

const fn p(m: u8) -> Pat {
    Pat {
        m,
        scope: PLAIN,
        caps: &[],
        op: Op::None,
        quote: false,
    }
}

impl Pat {
    const fn sc(mut self, kind: u16) -> Pat {
        self.scope = kind;
        self
    }
    const fn caps(mut self, caps: &'static [(u8, u16)]) -> Pat {
        self.caps = caps;
        self
    }
    const fn push(mut self, ctxs: &'static [u8]) -> Pat {
        self.op = Op::Push(ctxs);
        self
    }
    const fn set(mut self, ctxs: &'static [u8]) -> Pat {
        self.op = Op::Set(ctxs);
        self
    }
    const fn pop(mut self) -> Pat {
        self.op = Op::Pop;
        self
    }
}

/// A context's entry: a pattern, or another context's patterns.
enum It {
    P(Pat),
    I(&'static [It]),
    /// `variable-substitutions`, one search for its eleven patterns ([`SUBSTITUTIONS`]).
    V,
}

use It::{I, P};

const LINE_CONTINUATION: &[It] = &[P(p(X_LINE_CONT)
    .caps(&[(1, mk::CONTINUATION)])
    .push(&[MK_CONTINUED]))];
const POP_ON_LINE_END: &[It] = &[P(p(X_EOL).pop())];
const CONTINUATION_OR_POP: &[It] = &[I(LINE_CONTINUATION), I(POP_ON_LINE_END)];
const QUOTED_STRING: &[It] = &[
    P(p(X_SQ).sc(mk::STR_BEGIN).push(&[STRING_SINGLE])),
    P(p(X_DQ).sc(mk::STR_BEGIN).push(&[STRING_DOUBLE])),
];
const ESCAPE_LITERALS: &[It] = &[P(p(X_ESCAPE).sc(mk::ESCAPE))];
const COMMENTS_POP_ON_LINE_END: &[It] = &[I(LINE_CONTINUATION), P(p(X_EOL_NL).pop())];
const COMMENTS: &[It] = &[
    P(p(X_COMMENT_AHEAD).push(&[COMMENT_START])),
    P(p(X_HASH).sc(mk::COMMENT_MARK).push(&[MK_COMMENT])),
];
const HIGHLIGHT_PERCENT: &[It] = &[P(p(X_PERCENT).sc(mk::PERCENT))];
const HIGHLIGHT_WILDCARD: &[It] = &[P(p(X_STAR).sc(mk::WILDCARD))];

const CALL_CAPS: &[(u8, u16)] = &[(1, mk::BLOCK_BEGIN), (2, mk::BUILTIN)];
/// `variable-substitutions`' patterns in the grammar's order (`function-invocations` first):
/// the one [`substitution`] found is `found[5]`'s start. Their matcher is [`X_SUBST`].
static SUBSTITUTIONS: [Pat; 11] = [
    p(X_SUBST)
        .caps(&[(1, mk::BLOCK_BEGIN), (2, mk::CALL_LANG)])
        .push(&[CALL]),
    p(X_SUBST).caps(CALL_CAPS).push(&[CALL_PATSUBST]),
    p(X_SUBST).caps(CALL_CAPS).push(&[CALL_WILDCARD]),
    p(X_SUBST).caps(CALL_CAPS).push(&[CALL_INFO]),
    p(X_SUBST).caps(CALL_CAPS).push(&[INSIDE_FUNCTION_CALL]),
    p(X_SUBST).caps(CALL_CAPS).push(&[CALL_SHELL]),
    p(X_SUBST).sc(mk::BLOCK_BEGIN).push(&[SUBSTITUTION_PAREN]),
    p(X_SUBST).sc(mk::BLOCK_BEGIN).push(&[SUBSTITUTION_BRACE]),
    p(X_SUBST).sc(mk::AUTOMATIC),
    p(X_SUBST).sc(mk::ESCAPE),
    p(X_SUBST).sc(mk::VAR_PARAM).caps(&[(1, mk::SINGLE_CHAR)]),
];
const VARIABLE_SUBSTITUTIONS: &[It] = &[It::V];
const SUBSTITUTION_COMMON: &[It] = &[
    P(p(X_COLON).sc(mk::SUBST_MARK)),
    P(p(X_EQ).sc(mk::ASSIGN_MARK)),
    I(HIGHLIGHT_PERCENT),
    I(VARIABLE_SUBSTITUTIONS),
];
const CONTROL_FLOW: &[It] = &[
    P(p(X_VPATH_WS).caps(&[(1, mk::VPATH)]).push(&[VPATH])),
    P(p(X_VPATH_EOL)),
    P(p(X_INCLUDE).caps(&[(1, mk::IMPORT)]).push(&[INCLUDE])),
    P(p(X_IFDEF).sc(mk::CONDITIONAL).push(&[INSIDE_CONTROL_FLOW])),
    P(p(X_ELSE).caps(&[(1, mk::CONDITIONAL)]).push(&[ELSE])),
    P(p(X_ENDIF).caps(&[(1, mk::CONDITIONAL)]).push(&[ENDIF])),
];
const VARIABLE_DEFINITIONS: &[It] = &[
    P(p(X_OVERRIDE).caps(&[(1, mk::CONTROL)]).set(&[OVERRIDE])),
    P(p(X_DEFINE_WS).caps(&[(1, mk::CONTROL)]).push(&[DEFINE])),
    P(p(X_EXPORT).caps(&[(1, mk::CONTROL)]).push(&[EXPORT])),
    P(p(X_VAR_AHEAD).push(&[VARIABLE])),
];
const INSIDE_CONTROL_FLOW_P: &[It] = &[
    P(p(X_SQ).sc(mk::STR_BEGIN).push(&[STRING_SINGLE])),
    P(p(X_DQ).sc(mk::STR_BEGIN).push(&[STRING_DOUBLE])),
    P(p(X_OPEN).sc(mk::GROUP_BEGIN).push(&[CONTROL_GROUP])),
    P(p(X_CLOSE).sc(mk::STRAY_PAREN)),
    I(CONTINUATION_OR_POP),
];
const RECIPE_JUNCTION_P: &[It] = &[
    I(COMMENTS),
    P(p(X_RECIPE_IF)
        .caps(&[(1, mk::CONDITIONAL)])
        .push(&[RECIPE_IF])),
    P(p(X_SPACES_AHEAD).set(&[RECIPE_WITH_SPACES])),
    P(p(X_TAB_AHEAD).set(&[RECIPE_WITH_TABS])),
    P(p(X_BOL).pop()),
];
const RECIPE_COMMON: &[It] = &[I(CONTROL_FLOW), P(p(X_BLANK))];
const INSIDE_FUNCTION_CALL_P: &[It] = &[
    P(p(X_CLOSE).sc(mk::BLOCK_END).pop()),
    P(p(X_OPEN).push(&[PAREN_BALANCER])),
    P(p(X_COMMA).sc(mk::SEPARATOR)),
    P(p(X_CALL_QUOTE)),
    I(VARIABLE_SUBSTITUTIONS),
    I(QUOTED_STRING),
];
const STRING_BODY: &[It] = &[
    I(ESCAPE_LITERALS),
    I(LINE_CONTINUATION),
    I(VARIABLE_SUBSTITUTIONS),
];

const MAIN_P: &[It] = &[
    I(COMMENTS),
    I(VARIABLE_DEFINITIONS),
    P(p(X_RULE_AHEAD).push(&[EXPECT_RULE])),
    I(VARIABLE_SUBSTITUTIONS),
    I(CONTROL_FLOW),
    P(p(X_ENDEF_STRAY).caps(&[(1, mk::STRAY_ENDEF)])),
];
const STRING_SINGLE_P: &[It] = &[P(p(X_SQ).sc(mk::STR_END).pop()), I(STRING_BODY)];
const STRING_DOUBLE_P: &[It] = &[P(p(X_DQ).sc(mk::STR_END).pop()), I(STRING_BODY)];
const CONTROL_GROUP_P: &[It] = &[
    P(p(X_CLOSE).sc(mk::GROUP_END).pop()),
    I(VARIABLE_SUBSTITUTIONS),
    P(p(X_COMMA).sc(mk::SEPARATOR)),
];
const VPATH_P: &[It] = &[
    I(HIGHLIGHT_WILDCARD),
    P(p(X_WS1).set(&[VPATH_DIRS])),
    I(POP_ON_LINE_END),
];
const INCLUDE_P: &[It] = &[
    I(CONTINUATION_OR_POP),
    I(VARIABLE_SUBSTITUTIONS),
    I(COMMENTS),
];
const ELSE_P: &[It] = &[I(CONTROL_FLOW), I(COMMENTS), I(POP_ON_LINE_END)];
const ENDIF_P: &[It] = &[I(COMMENTS), I(POP_ON_LINE_END)];
const EXPECT_RULE_P: &[It] = &[
    I(LINE_CONTINUATION),
    I(VARIABLE_SUBSTITUTIONS),
    I(HIGHLIGHT_PERCENT),
    P(p(X_COLON_EQ_AHEAD).set(&[TARGET_VARIABLE])),
    P(p(X_COLON_AHEAD).set(&[RULE_COLON])),
];
const TARGET_VARIABLE_P: &[It] = &[P(p(X_COLONS).sc(mk::ASSIGN).set(&[TARGET_VARIABLE_VALUE]))];
const TARGET_VARIABLE_VALUE_P: &[It] = &[
    I(VARIABLE_DEFINITIONS),
    I(VARIABLE_SUBSTITUTIONS),
    I(CONTINUATION_OR_POP),
];
const RULE_COLON_P: &[It] = &[P(p(X_COLONS_WS)
    .caps(&[(1, mk::ASSIGN)])
    .set(&[PREREQUISITES]))];
const PREREQUISITES_P: &[It] = &[
    I(LINE_CONTINUATION),
    I(VARIABLE_SUBSTITUTIONS),
    I(HIGHLIGHT_PERCENT),
    P(p(X_HASH_AHEAD).set(&[RULE_COMMENT_START])),
    P(p(X_EOL).set(&[RECIPE_JUNCTION])),
    P(p(X_SEMI_AHEAD).set(&[RULE_SEMICOLON])),
];
const RULE_COMMENT_START_P: &[It] = &[P(p(X_HASH).sc(mk::COMMENT_MARK).set(&[RULE_COMMENT]))];
const RULE_COMMENT_P: &[It] = &[P(p(X_EOL).set(&[RECIPE_JUNCTION]))];
const RULE_SEMICOLON_P: &[It] = &[P(p(X_SEMI).sc(mk::TERMINATOR).set(&[RECIPE_INLINE]))];
const RECIPE_IF_P: &[It] = &[I(INSIDE_CONTROL_FLOW_P), I(RECIPE_JUNCTION_P)];
const RECIPE_INLINE_P: &[It] = &[
    I(RECIPE_JUNCTION_P),
    I(CONTROL_FLOW),
    P(p(X_EOL).set(&[RECIPE_JUNCTION])),
    P(p(X_EMPTY).push(&[SHELL])),
];
const RECIPE_WITH_SPACES_P: &[It] = &[
    P(p(X_RECIPE_SPACES)
        .caps(&[(2, mk::CONST_LANG)])
        .push(&[SHELL])),
    P(p(X_RECIPE_TAB2).caps(&[(1, mk::EXPECTED_SPACES), (2, mk::CONST_LANG)])),
    I(RECIPE_COMMON),
    P(p(X_NOT_SPACES).pop()),
];
const RECIPE_WITH_TABS_P: &[It] = &[
    P(p(X_RECIPE_TAB1).caps(&[(1, mk::CONST_LANG)]).push(&[SHELL])),
    P(p(X_RECIPE_SPACES).caps(&[(1, mk::EXPECTED_TAB), (2, mk::CONST_LANG)])),
    I(RECIPE_COMMON),
    P(p(X_NOT_TAB).pop()),
];
const SHELL_P: &[It] = &[
    P(p(X_HASH_WS)
        .caps(&[(1, super::k::COMMENT_BEGIN)])
        .set(&[SHELL_COMMENT])),
    P(p(X_EMPTY).set(&[SH_RECIPE])),
];
const MK_CONTINUED_P: &[It] = &[P(p(X_CONT_END).pop())];
const COMMENT_START_P: &[It] = &[P(p(X_HASH).sc(mk::COMMENT_MARK).set(&[MK_COMMENT]))];
const CALL_P: &[It] = &[
    P(p(X_CLOSE_AHEAD).set(&[INSIDE_FUNCTION_CALL])),
    P(p(X_COMMA_AHEAD).set(&[CALL_COMMA])),
    I(VARIABLE_SUBSTITUTIONS),
];
const CALL_COMMA_P: &[It] = &[P(p(X_COMMA).sc(mk::SEPARATOR).set(&[INSIDE_FUNCTION_CALL]))];
const CALL_PATSUBST_P: &[It] = &[I(HIGHLIGHT_PERCENT), I(INSIDE_FUNCTION_CALL_P)];
const CALL_WILDCARD_P: &[It] = &[I(INSIDE_FUNCTION_CALL_P), I(HIGHLIGHT_WILDCARD)];
const CALL_INFO_P: &[It] = &[
    P(p(X_CLOSE).sc(mk::BLOCK_END).pop()),
    P(p(X_OPEN).push(&[PAREN_BALANCER])),
    P(p(X_COMMA).sc(mk::SEPARATOR)),
    I(VARIABLE_SUBSTITUTIONS),
];
const CALL_SHELL_P: &[It] = &[
    P(p(X_CLOSE).sc(mk::BLOCK_END).pop()),
    P(p(X_EMPTY).push(&[SH_CALL])),
];
const OVERRIDE_P: &[It] = &[
    P(p(X_DEFINE_WORD).sc(mk::CONTROL).push(&[DEFINE])),
    I(VARIABLE_DEFINITIONS),
    I(CONTINUATION_OR_POP),
];
const EXPORT_P: &[It] = &[
    I(CONTINUATION_OR_POP),
    I(VARIABLE_SUBSTITUTIONS),
    P(p(X_ASSIGN_OP).sc(mk::ASSIGN).set(&[VALUE, EAT_WHITESPACE])),
];
const VARIABLE_P: &[It] = &[
    P(p(X_SHELL_ASSIGN_AHEAD).set(&[SHELL_ASSIGN])),
    P(p(X_ASSIGN_AHEAD).set(&[ASSIGN])),
    I(VARIABLE_SUBSTITUTIONS),
];
const SHELL_ASSIGN_P: &[It] = &[P(p(X_BANG_EQ).sc(mk::ASSIGN).set(&[SH_ASSIGN]))];
const ASSIGN_P: &[It] = &[
    P(p(X_ASSIGN_OP_BANG)
        .sc(mk::ASSIGN)
        .set(&[VALUE_MAYBE_SHELL, EAT_WHITESPACE])),
    I(VARIABLE_SUBSTITUTIONS),
    I(CONTINUATION_OR_POP),
];
const PAREN_BALANCER_P: &[It] = &[P(p(X_CLOSE).pop()), I(VARIABLE_SUBSTITUTIONS)];
const EAT_WHITESPACE_P: &[It] = &[P(p(X_EAT_WS).pop())];
const VALUE_MAYBE_SHELL_P: &[It] = &[
    P(Pat {
        quote: true,
        ..p(X_SHELL_C).caps(&[(1, mk::UNQUOTED), (2, mk::INTERP_BEGIN)])
    }),
    P(p(X_EMPTY).set(&[VALUE])),
];
const EMBEDDED_SCRIPT_SQ_P: &[It] = &[P(p(X_QUOTE_SQ).sc(mk::INTERP_END).pop())];
const EMBEDDED_SCRIPT_DQ_P: &[It] = &[P(p(X_QUOTE_DQ).sc(mk::INTERP_END).pop())];
const EMBEDDED_SCRIPT_BT_P: &[It] = &[P(p(X_QUOTE_BT).sc(mk::INTERP_END).pop())];
const VALUE_P: &[It] = &[
    I(ESCAPE_LITERALS),
    P(p(X_HASH_AHEAD).set(&[VALUE_COMMENT_START])),
    I(VARIABLE_SUBSTITUTIONS),
    I(CONTINUATION_OR_POP),
];
const DEFINE_P: &[It] = &[
    P(p(X_DEFINE_OP_AHEAD).set(&[DEFINE_ASSIGN])),
    P(p(X_NL).set(&[DEFINE_VALUE])),
    I(VARIABLE_SUBSTITUTIONS),
];
const DEFINE_ASSIGN_P: &[It] = &[P(p(X_DEFINE_ASSIGN)
    .caps(&[(1, mk::ASSIGN)])
    .set(&[DEFINE_VALUE]))];
const DEFINE_VALUE_P: &[It] = &[
    P(p(X_ENDEF).caps(&[(1, mk::CONTROL)]).pop()),
    P(p(X_DEFINE_NESTED).push(&[DEFINE_NESTED])),
];
const DEFINE_NESTED_P: &[It] = &[P(p(X_ENDEF).pop())];
const SUBSTITUTION_PAREN_P: &[It] = &[
    P(p(X_CLOSE).sc(mk::BLOCK_END).pop()),
    I(SUBSTITUTION_COMMON),
];
const SUBSTITUTION_BRACE_P: &[It] = &[
    P(p(X_BRACE_CLOSE).sc(mk::BLOCK_END).pop()),
    I(SUBSTITUTION_COMMON),
];

/// Each context's patterns, by its index.
const PATS: [&[It]; COUNT] = [
    MAIN_P,
    INSIDE_CONTROL_FLOW_P,
    STRING_SINGLE_P,
    STRING_DOUBLE_P,
    CONTROL_GROUP_P,
    VPATH_P,
    POP_ON_LINE_END, // VPATH_DIRS
    INCLUDE_P,
    ELSE_P,
    ENDIF_P,
    EXPECT_RULE_P,
    TARGET_VARIABLE_P,
    TARGET_VARIABLE_VALUE_P,
    RULE_COLON_P,
    PREREQUISITES_P,
    RULE_COMMENT_START_P,
    RULE_COMMENT_P,
    RULE_SEMICOLON_P,
    RECIPE_JUNCTION_P,
    RECIPE_IF_P,
    RECIPE_INLINE_P,
    RECIPE_WITH_SPACES_P,
    RECIPE_WITH_TABS_P,
    SHELL_P,
    COMMENTS_POP_ON_LINE_END, // SHELL_COMMENT
    MK_CONTINUED_P,
    COMMENT_START_P,
    COMMENTS_POP_ON_LINE_END, // MK_COMMENT
    INSIDE_FUNCTION_CALL_P,
    CALL_P,
    CALL_COMMA_P,
    CALL_PATSUBST_P,
    CALL_WILDCARD_P,
    CALL_INFO_P,
    CALL_SHELL_P,
    OVERRIDE_P,
    EXPORT_P,
    VARIABLE_P,
    SHELL_ASSIGN_P,
    ASSIGN_P,
    PAREN_BALANCER_P,
    EAT_WHITESPACE_P,
    VALUE_MAYBE_SHELL_P,
    EMBEDDED_SCRIPT_SQ_P,
    EMBEDDED_SCRIPT_DQ_P,
    EMBEDDED_SCRIPT_BT_P,
    VALUE_P,
    COMMENT_START_P, // VALUE_COMMENT_START: the same pattern
    DEFINE_P,
    DEFINE_ASSIGN_P,
    DEFINE_VALUE_P,
    DEFINE_NESTED_P,
    SUBSTITUTION_PAREN_P,
    SUBSTITUTION_BRACE_P,
];

/// `variable-substitutions` in a flattened list: [`SUBSTITUTIONS`]' one search.
const SUBSTITUTION_ENTRY: Pat = p(X_SUBST);

/// The most patterns a context reaches through its includes.
const FLAT_MAX: usize = 16;

/// A context's patterns, its includes followed, in order.
struct Flat {
    n: usize,
    pats: [&'static Pat; FLAT_MAX],
}

const fn flatten(its: &'static [It], into: &mut Flat) {
    let mut i = 0;
    while i < its.len() {
        match &its[i] {
            P(pat) => {
                assert!(into.n < FLAT_MAX, "a context's patterns fit");
                into.pats[into.n] = pat;
                into.n += 1;
            }
            I(inner) => flatten(inner, into),
            It::V => {
                assert!(into.n < FLAT_MAX, "a context's patterns fit");
                into.pats[into.n] = &SUBSTITUTION_ENTRY;
                into.n += 1;
            }
        }
        i += 1;
    }
}

/// Each context's flattened patterns, by its index.
static FLAT: [Flat; COUNT] = {
    let mut t = [const {
        Flat {
            n: 0,
            pats: [&SUBSTITUTION_ENTRY; FLAT_MAX],
        }
    }; COUNT];
    let mut c = 0;
    while c < COUNT {
        flatten(PATS[c], &mut t[c]);
        c += 1;
    }
    t
};

/// The contexts `bash -c` pushes, by its quote.
static EMBEDDED: [&[u8]; 3] = [
    &[EMBEDDED_SCRIPT_SQ, SH_SQ],
    &[EMBEDDED_SCRIPT_DQ, SH_DQ],
    &[EMBEDDED_SCRIPT_BT, SH_BT],
];

/// The best match a context's patterns have found so far at a step.
#[derive(Clone, Copy)]
struct Best {
    min: usize,
    pat: Option<&'static Pat>,
    would_loop: bool,
}

impl Best {
    fn new() -> Self {
        Self {
            min: usize::MAX,
            pat: None,
            would_loop: false,
        }
    }
}

// ---- the frames

/// What a pattern does to the stack.
#[derive(Clone, Copy)]
enum Op<'c> {
    None,
    Push(&'c [u8]),
    Set(&'c [u8]),
    Pop,
}

#[derive(Clone, Copy)]
struct Frame {
    ctx: u8,
    /// Inside a make substitution's `variable.parameter` ([`TWIN_MK`]).
    param: bool,
    /// The kind of the text in it that no pattern takes: its meta content scope's, its meta
    /// scope's, or the one below's.
    base: u16,
}

const FRAME: Frame = Frame {
    ctx: MAIN,
    param: false,
    base: PLAIN,
};

/// The frames a scan holds.
const MAX: usize = 128;
/// syntect's limit: a push at this depth (its stack, ours plus its `__start`) is refused.
const SYNTECT_DEPTH: usize = 99;

struct Scan<'a, 'o, 'p> {
    b: &'a [u8],
    /// The frames below `depth` are written; those above are not.
    stack: [MaybeUninit<Frame>; MAX],
    depth: usize,
    /// Where the spans so far end.
    emitted: usize,
    /// syntect's loop guard: where the last push that matched nothing was, and the depth it
    /// left.
    push_at: (usize, usize),
    /// Whether that holds at the place being matched.
    guard: bool,
    exact: bool,
    out: &'o mut Out<'p>,
    mk: Mk<'a>,
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

/// A shell region's make tokens on a line, in order: where each starts and ends.
/// A token's byte in the shell's text, or none.
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

/// Whether bash's kind is an option's: a make token right after it is inside the option.
fn in_option(kind: u16) -> bool {
    kind != PLAIN && kind & super::TWIN != 0
        || matches!(
            kind,
            super::k::OPTION | super::k::OPTION_MARK | super::k::OPTION_ASSIGN
        )
}

/// Lex `b`, a line of a Makefile without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let n = b.len();
    let mut small = [0u8; 256];
    let heap: Vec<u8>;
    let buf: &[u8] = if n < small.len() {
        small[..n].copy_from_slice(b);
        small[n] = b'\n';
        &small[..=n]
    } else {
        let mut v = Vec::with_capacity(n + 1);
        v.extend_from_slice(b);
        v.push(b'\n');
        heap = v;
        &heap
    };
    // SAFETY: a line's text, a `str`, then a `\n`
    let text = unsafe { std::str::from_utf8_unchecked(buf) };
    let mut s = Scan {
        b: &buf[..n],
        stack: [const { MaybeUninit::uninit() }; MAX],
        depth: 1,
        emitted: 0,
        push_at: (0, 0),
        guard: false,
        exact: state.is_exact(),
        out,
        mk: Mk {
            l: Line::new(text),
            present: b.iter().fold(0, |set, &c| set | BIT[usize::from(c)]),
            valid: [0; 2],
            span: [const { MaybeUninit::uninit() }; MATCHERS],
            found: [const { MaybeUninit::uninit() }; MATCHERS],
        },
        sh: (0, State::default()),
    };
    s.stack[0] = MaybeUninit::new(FRAME);
    s.load(state);
    s.run(0, 0);
    s.fill(n);
    *state = s.save();
}

impl Scan<'_, '_, '_> {
    // ---- the line

    fn at(&self, i: usize) -> u8 {
        match self.b.get(i) {
            Some(&c) => c,
            None if i == self.b.len() => b'\n',
            None => 0,
        }
    }

    /// The place after the character at `p`.
    fn next_char(&self, p: usize) -> usize {
        if p >= self.b.len() {
            p + 1
        } else {
            p + char_len(self.b[p])
        }
    }

    // ---- the stack

    fn top(&self) -> &Frame {
        self.at_frame(self.depth - 1)
    }

    /// The frame at `i`, below `depth`.
    fn at_frame(&self, i: usize) -> &Frame {
        debug_assert!(i < self.depth);
        // SAFETY: every frame below `depth` was written (`push_frame`, and the bottom one in
        // `line`); `depth` never passes `MAX`
        unsafe { self.stack.get_unchecked(i).assume_init_ref() }
    }

    fn push_frame(&mut self, ctx: u8) {
        let below = *self.top();
        let (meta, content) = INFO[usize::from(ctx)];
        let base = if ctx == EAT_WHITESPACE {
            // `clear_scopes: 1`, where what it clears is all there is
            PLAIN
        } else if content != PLAIN {
            content
        } else if meta != PLAIN {
            meta
        } else {
            below.base
        };
        let f = Frame {
            ctx,
            param: below.param || matches!(ctx, SUBSTITUTION_PAREN | SUBSTITUTION_BRACE),
            base,
        };
        if self.depth == MAX {
            self.stack.copy_within(1.., 0);
            self.depth -= 1;
            self.exact = false;
        }
        self.stack[self.depth] = MaybeUninit::new(f);
        self.depth += 1;
    }

    fn pop_frame(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
    }

    // ---- spans

    fn put(&mut self, s: usize, e: usize, kind: u16) {
        let e = e.min(self.b.len());
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

    /// The text up to `to` that no pattern took, in the top context's kind.
    fn fill(&mut self, to: usize) {
        let s = self.emitted;
        if to > s {
            let base = self.top().base;
            self.put(s, to, base);
        }
    }

    /// A match's text `s..e` in `base`, its groups in theirs, the innermost on top.
    fn paint(&mut self, s: usize, e: usize, base: u16, caps: &[(usize, usize, u16)]) {
        if caps.is_empty() {
            return self.put(s, e, base);
        }
        let mut at = s;
        while at < e {
            // the innermost group holding `at`, and where the next boundary is
            let mut kind = base;
            let mut len = usize::MAX;
            let mut next = e;
            for &(cs, ce, ck) in caps {
                if cs > at {
                    next = next.min(cs);
                } else if ce > at {
                    next = next.min(ce);
                    if ce - cs < len {
                        len = ce - cs;
                        kind = ck;
                    }
                }
            }
            self.put(at, next, kind);
            at = next;
        }
    }

    /// Do a match `s..e`: its scope (or [`PLAIN`]), its groups' scopes, its operation. `None`
    /// when it is a pop that would loop.
    fn exec(
        &mut self,
        s: usize,
        e: usize,
        scope: u16,
        caps: &[(usize, usize, u16)],
        op: Op,
    ) -> Option<usize> {
        if s == e && matches!(op, Op::Pop) && self.guard {
            return None;
        }
        if let Op::Push(ctxs) = op
            && self.depth >= SYNTECT_DEPTH
            && !ctxs.is_empty()
        {
            return None;
        }
        self.fill(s);
        // the text's kind where no scope of the pattern's is: the meta scope of the contexts
        // it opens, for a push or a set; the closing context's, for a pop
        let base = match op {
            Op::Push(ctxs) | Op::Set(ctxs) => ctxs
                .iter()
                .rev()
                .map(|&c| INFO[usize::from(c)].0)
                .find(|&m| m != PLAIN)
                .unwrap_or(self.top().base),
            Op::Pop => {
                let ctx = self.top().ctx;
                let meta = INFO[usize::from(ctx)].0;
                if ctx == EAT_WHITESPACE {
                    // what it cleared comes back after the match
                    PLAIN
                } else if meta != PLAIN {
                    meta
                } else {
                    self.at_frame(self.depth.saturating_sub(2)).base
                }
            }
            Op::None => self.top().base,
        };
        let base = if scope != PLAIN { scope } else { base };
        self.paint(s, e, base, caps);
        match op {
            Op::None => {}
            Op::Pop => self.pop_frame(),
            Op::Push(ctxs) => {
                if s == e {
                    self.push_at = (e, self.depth + 1);
                }
                for &c in ctxs {
                    self.push_frame(c);
                }
            }
            Op::Set(ctxs) => {
                self.pop_frame();
                for &c in ctxs {
                    self.push_frame(c);
                }
            }
        }
        Some(e)
    }

    fn push(&mut self, s: usize, e: usize, scope: u16, ctxs: &[u8]) -> Option<usize> {
        self.exec(s, e, scope, &[], Op::Push(ctxs))
    }

    // ---- the run

    /// The line from `p` until the frames are back to `floor` (where they end, `Some`) or the
    /// line ends (`None`).
    fn run(&mut self, mut p: usize, floor: usize) -> Option<usize> {
        let n = self.b.len();
        loop {
            if self.depth <= floor {
                return Some(p);
            }
            self.guard = self.push_at == (p, self.depth);
            let ctx = self.top().ctx;
            let step = if ctx >= SH_RECIPE {
                self.shell(p, ctx)
            } else {
                self.make_step(p, ctx)
            };
            match step {
                Ok(e) => p = e,
                Err(next) => {
                    if p > n || next > n + 1 {
                        return None;
                    }
                    p = next.max(p + 1).min(n + 1);
                }
            }
        }
    }

    // ---- shell regions

    /// The shell region on top, from `p`: its make tokens run on make's frames, each a byte in
    /// the shell's text or none ([`Cuts::stand_in`]), which bash's scanner lexes from the state
    /// the region kept; the region popped where it ends on the line, else kept with bash's state
    /// (the module's doc).
    #[inline(never)]
    fn shell(&mut self, p: usize, ctx: u8) -> Result<usize, usize> {
        let b = self.b;
        let n = b.len();
        let d = self.depth;
        let mut st = if self.sh.0 == d {
            self.sh.0 = 0;
            std::mem::take(&mut self.sh.1)
        } else {
            State::default()
        };
        if p >= n {
            // nothing of the region on the line: a recipe's and `!=`'s end, the others go on
            if matches!(ctx, SH_RECIPE | SH_ASSIGN) {
                self.pop_frame();
                return Ok(n);
            }
            self.keep_shell(d, st);
            return Err(usize::MAX);
        }
        // the make tokens, and where the region's text on the line ends; the tokens' text that
        // no make scope takes is bash's at the token's byte, so the region's kind is out of it
        let frame = *self.top();
        let region = frame.base;
        self.stack[d - 1] = MaybeUninit::new(Frame {
            base: PLAIN,
            ..frame
        });
        let bit = 1u8 << (ctx - SH_RECIPE);
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
                b')' if ctx == SH_ASSIGN => break (t, Stop::Past(t)),
                b')' => break (t, Stop::At(t)),
                b'\'' | b'"' | b'`' if ctx != SH_CALL => break (t, Stop::At(t)),
                b'$' => {
                    self.emitted = t;
                    self.substitution_at(t)
                }
                c => {
                    self.emitted = t;
                    match c {
                        b'\'' => self.push(t, t + 1, mk::STR_BEGIN, &[STRING_SINGLE]),
                        b'"' => self.push(t, t + 1, mk::STR_BEGIN, &[STRING_DOUBLE]),
                        _ => self.push(t, t + 1, PLAIN, &[PAREN_BALANCER]),
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
        if ctx == SH_RECIPE
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
                self.pop_frame();
                Ok(t)
            }
            Stop::Past(t) => {
                let below = self.at_frame(d - 2).base;
                self.pop_frame();
                self.put(t, t + 1, below);
                Ok(t + 1)
            }
            Stop::Open => {
                if self.sh.0 == 0 {
                    self.sh = (d, st);
                }
                Err(usize::MAX)
            }
            Stop::Eol => {
                // a recipe goes on past a `\` ending it, as bash's `\\\n` keeps bat's shell;
                // `!=`'s ends with its line; `$(shell …)` and an embed go on
                let goes_on = match ctx {
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
                    Err(usize::MAX)
                } else {
                    self.pop_frame();
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
    #[allow(clippy::too_many_arguments)]
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
            let ctx = state.level(i);
            if usize::from(ctx) >= CONTEXTS {
                break;
            }
            self.push_frame(ctx);
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
                .find(|&d| self.at_frame(d).ctx >= SH_RECIPE)
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
            inexact: !self.exact || from > 1,
            arg: frames as u16,
            ..State::default()
        };
        for d in from..self.depth {
            state.set_level(d - from, self.at_frame(d).ctx);
        }
        let (d, st) = &self.sh;
        if *d != 0 && *d <= self.depth && self.at_frame(*d - 1).ctx >= SH_RECIPE {
            let held = st.held();
            if frames + held <= HELD {
                for i in 0..held {
                    state.set_level(frames + i, st.level(i));
                }
                state.depth += held as u8;
                state.inexact |= !st.is_exact();
            } else {
                state.inexact = true;
            }
        }
        state
    }
}

impl Scan<'_, '_, '_> {
    /// Where a make matcher's leftmost match from `p` starts and ends, kept for the line; the
    /// match itself stays in the memo, for the pattern that wins.
    fn search(&mut self, id: u8, p: usize) -> Option<(u32, u32)> {
        let mk = &mut self.mk;
        let i = usize::from(id);
        // a byte the line lacks rules a match out from anywhere on it
        let need = REQUIRE[i];
        if need != 0 && mk.present & BIT[usize::from(need)] == 0 {
            return None;
        }
        if mk.valid[i >> 6] >> (i & 63) & 1 == 1 {
            // SAFETY: a span whose bit is set was written
            match unsafe { mk.span[i].assume_init() } {
                (NO_MATCH, _) => return None,
                (s, e) if s as usize >= p => return Some((s, e)),
                _ => {}
            }
        }
        mk.valid[i >> 6] |= 1 << (i & 63);
        let (find, arg) = FIND[i];
        match find(&mk.l, p, arg) {
            Some(m) => {
                mk.span[i] = MaybeUninit::new(m.c[0]);
                mk.found[i] = MaybeUninit::new(m);
                Some(m.c[0])
            }
            None => {
                mk.span[i] = MaybeUninit::new((NO_MATCH, 0));
                None
            }
        }
    }

    /// The match a matcher's span stands for.
    fn found(&self, id: u8) -> &M {
        let mk = &self.mk;
        // SAFETY: only read for a matcher whose span was a match, which wrote it
        unsafe { mk.found[usize::from(id)].assume_init_ref() }
    }

    /// Weigh `pat`'s match from `p` against the best so far: true when it is the one, at `p`.
    fn cand(&mut self, p: usize, b: &mut Best, pat: &'static Pat) -> bool {
        let Some(se) = self.search(pat.m, p) else {
            return false;
        };
        self.weigh(p, b, pat, se)
    }

    /// `variable-substitutions`' match from `p`, as one of its patterns.
    fn cand_substitution(&mut self, p: usize, b: &mut Best) -> bool {
        let Some(se) = self.search(X_SUBST, p) else {
            return false;
        };
        let pat = &SUBSTITUTIONS[self.found(X_SUBST).c[5].0 as usize];
        self.weigh(p, b, pat, se)
    }

    /// Weigh `pat`'s match `s..e` against the best so far: true when it is the one, at `p`.
    fn weigh(&mut self, p: usize, b: &mut Best, pat: &'static Pat, (s, e): (u32, u32)) -> bool {
        // a match that does nothing is passed over
        if matches!(pat.op, Op::None) && s == e {
            return false;
        }
        let (s, e) = (s as usize, e as usize);
        if s < b.min || (s == b.min && b.would_loop) {
            b.min = s;
            b.would_loop = self.guard && e <= p && matches!(pat.op, Op::Pop);
            b.pat = Some(pat);
            return s == p && !b.would_loop;
        }
        false
    }

    fn walk(&mut self, its: &'static [It], p: usize, b: &mut Best) -> bool {
        for it in its {
            let found = match it {
                I(inner) => self.walk(inner, p, b),
                P(pat) => self.cand(p, b, pat),
                It::V => self.cand_substitution(p, b),
            };
            if found {
                return true;
            }
        }
        false
    }

    /// Do the best match: `Ok` with where it ends, or `Err` with the next place to match.
    fn take(&mut self, p: usize, b: Best) -> Result<usize, usize> {
        let Some(pat) = b.pat else {
            return Err(usize::MAX);
        };
        if b.would_loop {
            return Err(self.next_char(p));
        }
        let m = *self.found(pat.m);
        let (s, e) = (m.s(), m.e());
        if s > p {
            // the loop guard is about where the step started
            self.guard = false;
        }
        let mut caps = [(0, 0, PLAIN); 2];
        let mut n = 0;
        for &(group, kind) in pat.caps {
            let (cs, ce) = m.c[usize::from(group)];
            if cs != u32::MAX && cs < ce {
                caps[n] = (cs as usize, ce as usize, kind);
                n += 1;
            }
        }
        let op = if pat.quote {
            let q = self.at(m.c[2].0 as usize);
            Op::Push(
                EMBEDDED[match q {
                    b'\'' => 0,
                    b'"' => 1,
                    _ => 2,
                }],
            )
        } else {
            pat.op
        };
        self.exec(s, e, pat.scope, &caps[..n], op).ok_or(usize::MAX)
    }

    /// A step in a make context: syntect's leftmost match of its patterns.
    #[inline(never)]
    fn make_step(&mut self, p: usize, ctx: u8) -> Result<usize, usize> {
        // none of `main`'s patterns matches in a line's `\n` alone: those at its start need
        // `^`, the others text
        if ctx == MAIN && p >= self.b.len() {
            return Err(usize::MAX);
        }
        let mut b = Best::new();
        let flat = &FLAT[usize::from(ctx)];
        for &pat in &flat.pats[..flat.n] {
            let found = if pat.m == X_SUBST {
                self.cand_substitution(p, &mut b)
            } else {
                self.cand(p, &mut b, pat)
            };
            if found {
                break;
            }
        }
        self.take(p, b)
    }

    /// `variable-substitutions` at `p`.
    fn substitution_at(&mut self, p: usize) -> Option<usize> {
        if self.at(p) != b'$' || p >= self.b.len() {
            return None;
        }
        let mut b = Best::new();
        self.walk(VARIABLE_SUBSTITUTIONS, p, &mut b);
        if b.pat.is_some() && b.min == p && !b.would_loop {
            return self.take(p, b).ok();
        }
        None
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
}
