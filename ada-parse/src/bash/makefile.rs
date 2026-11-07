//! Makefiles on bash's scanner (ADR-40), in place of the port of syntect's machine that ran
//! them (`make`, G-002). bat's `Makefile` is the reference, its contexts on the scanner's
//! stack beside bash's, and recipes, `!=`, `$(shell …)` and `bash -c '…'` lines are bash's
//! contexts, scanned in place as a shell script's are.
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
//! Where bat embeds the shell with a `with_prototype`, make's patterns ride on every shell
//! context above, ahead of the shell's own: a frame's embedding (`W_*`) says which, and the
//! scan tries them in place before the context's patterns, and passes no run where one of them
//! may start. What a make pattern pushes there sees none of them (syntect's `proto_starts`).
//!
//! **Kinds are leaves**, as bash's are: bash's kinds below `0x100`, make's from there
//! ([`MK`]). In the 14 built-in themes a byte's rule is that of its deepest scope that isn't
//! `meta.*` or `source.*`, but where a `variable.parameter` scope around it outranks a
//! `variable.*` leaf: an option's ([`super::TWIN`]) or a make substitution's ([`TWIN_MK`]).

use std::mem::MaybeUninit;

use super::matchers as g;
use super::scan::Line;
use super::scan::{self, M, R};
use crate::emit::{Push, Rules, Run, Runs, Sink};
use crate::{Out, State};

use super::{
    FRAME, Frame, Info, MK_FIRST, Op, PLAIN, SRC_ASSIGN, SRC_CALL, SRC_EMBED_BT, SRC_EMBED_DQ,
    SRC_EMBED_SQ, SRC_RECIPE, Scan, info as ctx_info,
};

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
        $(pub const $c: u8 = MK_FIRST + CtxId::$c as u8;)*
        /// How many contexts the Makefile grammar has.
        pub const COUNT: usize = CtxId::Count as usize;
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

/// The make contexts' entries of the scanner's table: no shell prototype, and the leaf of
/// their meta and meta content scopes.
pub(super) const fn info<const N: usize>(mut t: [Info; N]) -> [Info; N] {
    let mut c = MK_FIRST as usize;
    while c < N {
        t[c] = ctx_info(false, PLAIN, PLAIN);
        c += 1;
    }
    macro_rules! set {
        ($($c:ident: $meta:expr, $content:expr;)*) => {
            $(t[$c as usize] = ctx_info(false, $meta, $content);)*
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
}

// ---- embeddings: the make patterns that come first in the shell contexts above

const W_RECIPE: u8 = 1;
const W_CALL: u8 = 2;
const W_ASSIGN: u8 = 3;
const W_EMBED_SQ: u8 = 4;
const W_EMBED_DQ: u8 = 5;
const W_EMBED_BT: u8 = 6;

/// The embedding a shell's `main` sets, as the Makefile embeds it in `src`.
pub(super) const fn with_of(src: u8) -> u8 {
    match src {
        SRC_RECIPE => W_RECIPE,
        SRC_CALL => W_CALL,
        SRC_ASSIGN => W_ASSIGN,
        SRC_EMBED_SQ => W_EMBED_SQ,
        SRC_EMBED_DQ => W_EMBED_DQ,
        _ => W_EMBED_BT,
    }
}

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
    P(p(X_EMPTY).set(&[SRC_RECIPE])),
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
    P(p(X_EMPTY).push(&[SRC_CALL])),
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
const SHELL_ASSIGN_P: &[It] = &[P(p(X_BANG_EQ).sc(mk::ASSIGN).set(&[SRC_ASSIGN]))];
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
    &[EMBEDDED_SCRIPT_SQ, SRC_EMBED_SQ],
    &[EMBEDDED_SCRIPT_DQ, SRC_EMBED_DQ],
    &[EMBEDDED_SCRIPT_BT, SRC_EMBED_BT],
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

/// Bytes an embedding's make patterns may start on, a bit per embedding.
static TRIGGER: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'$' as usize] = (1 << W_RECIPE) | (1 << W_CALL) | (1 << W_ASSIGN);
    t[b'\\' as usize] = 1 << W_RECIPE;
    t[b'#' as usize] = 1 << W_RECIPE;
    t[b')' as usize] = (1 << W_CALL) | (1 << W_ASSIGN);
    t[b'(' as usize] = 1 << W_CALL;
    t[b'"' as usize] = (1 << W_CALL) | (1 << W_EMBED_DQ);
    t[b'\'' as usize] = (1 << W_CALL) | (1 << W_EMBED_SQ);
    t[b'`' as usize] = 1 << W_EMBED_BT;
    t
};

/// Lex `b`, a line of a Makefile without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    line_on(b, state, out, Push);
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-075, X-068's sink): [`crate::lex_runs`] for
/// a Makefile.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    line_on(b, state, out, Runs::new(rules, runs)).end_line(b.len());
}

/// [`line`]'s scan on the sink `k`, which it returns as the line leaves it.
#[inline(always)]
fn line_on<K: Sink>(b: &[u8], state: &mut State, out: &mut Out, k: K) -> K {
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
    let mut mk = Mk {
        l: Line::new(text),
        present: b.iter().fold(0, |set, &c| set | BIT[usize::from(c)]),
        valid: [0; 2],
        span: [const { MaybeUninit::uninit() }; MATCHERS],
        found: [const { MaybeUninit::uninit() }; MATCHERS],
    };
    let mut s = Scan::new(&buf[..n], state, out, Some(&mut mk), k);
    s.stack[0] = MaybeUninit::new(Frame { ctx: MAIN, ..FRAME });
    s.load(state);
    s.run::<true>();
    s.fill(s.b.len());
    *state = s.save();
    s.k
}

/// A block of a Makefile on one scan opened from `state` ([`crate::Carry`], X-073): bash's kept
/// scan, its [`Mk`] made once, a line copied with its `\n` into a buffer kept for the block;
/// `state` left as the last line leaves it.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut mk = Mk {
        l: Line::new(""),
        present: 0,
        valid: [0; 2],
        span: [const { MaybeUninit::uninit() }; MATCHERS],
        found: [const { MaybeUninit::uninit() }; MATCHERS],
    };
    let mut s = Scan::new(b"", state, &mut out, Some(&mut mk), Push);
    s.stack[0] = MaybeUninit::new(Frame { ctx: MAIN, ..FRAME });
    s.load(state);
    let mut c = Carried { s, buf: Vec::new() };
    let rest = pass(&mut c);
    *state = c.s.save();
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`], X-075).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    pass: &mut dyn FnMut(&mut dyn crate::CarryRuns) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut mk = Mk {
        l: Line::new(""),
        present: 0,
        valid: [0; 2],
        span: [const { MaybeUninit::uninit() }; MATCHERS],
        found: [const { MaybeUninit::uninit() }; MATCHERS],
    };
    let k = Runs::new(rules, runs);
    let mut s = Scan::new(b"", state, &mut out, Some(&mut mk), k);
    s.stack[0] = MaybeUninit::new(Frame { ctx: MAIN, ..FRAME });
    s.load(state);
    let mut c = Carried { s, buf: Vec::new() };
    let rest = pass(&mut c);
    *state = c.s.save();
    rest
}

/// A Makefile's block: the kept scan, and its line with a `\n` after it.
struct Carried<'a, 'o, 'p, K: Sink> {
    s: Scan<'a, 'o, 'p, K>,
    buf: Vec<u8>,
}

impl crate::Carry for Carried<'_, '_, '_, Push> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.s.out.0.clear();
        self.carry(b);
        self.s.out.0
    }
}

impl crate::CarryRuns for Carried<'_, '_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        self.s.k.start_line();
        self.carry(b);
        self.s.k.end_line(b.len())
    }
}

impl<K: Sink> Carried<'_, '_, '_, K> {
    /// `b`, the block's next line, lexed on the kept scan; its line and `Mk`'s empty ones again
    /// after.
    #[inline(always)]
    fn carry(&mut self, b: &[u8]) {
        let n = b.len();
        self.buf.clear();
        self.buf.reserve(n + 1);
        self.buf.extend_from_slice(b);
        self.buf.push(b'\n');
        // SAFETY: the scan's line and `Mk`'s are set back to empty ones below, before the
        // buffer is written again or dropped
        let buf = unsafe { crate::line_for(&self.buf) };
        // SAFETY: a line's text, a `str`, then a `\n`
        let text = unsafe { std::str::from_utf8_unchecked(buf) };
        // SAFETY: a Makefile's scan has its `Mk`
        let mk = unsafe { self.s.mk.as_deref_mut().unwrap_unchecked() };
        mk.l = Line::new(text);
        mk.present = b.iter().fold(0, |set, &c| set | BIT[usize::from(c)]);
        mk.valid = [0; 2];
        self.s.carry::<true>(&buf[..n]);
        // SAFETY: as above
        unsafe { self.s.mk.as_deref_mut().unwrap_unchecked() }.l = Line::new("");
    }
}

impl<K: Sink> Scan<'_, '_, '_, K> {
    /// Where a make matcher's leftmost match from `p` starts and ends, kept for the line; the
    /// match itself stays in the memo, for the pattern that wins.
    fn search(&mut self, id: u8, p: usize) -> Option<(u32, u32)> {
        let mk = self.mk.as_deref_mut()?;
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
        let mk = self.mk.as_deref().expect("a Makefile's line");
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
        self.exec(s, e, pat.scope, &caps[..n], op, 0)
            .ok_or(usize::MAX)
    }

    /// [`Scan::step`] in a make context: syntect's leftmost match of its patterns.
    #[inline(never)]
    pub(super) fn make_step(&mut self, p: usize, ctx: u8) -> Result<usize, usize> {
        // none of `main`'s patterns matches in a line's `\n` alone: those at its start need
        // `^`, the others text
        if ctx == MAIN && p >= self.b.len() {
            return Err(usize::MAX);
        }
        let mut b = Best::new();
        let flat = &FLAT[usize::from(ctx - MK_FIRST)];
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

    /// The make patterns that come first in a shell context of embedding `with`, at `p`.
    #[inline(never)]
    pub(super) fn with_proto(&mut self, p: usize, with: u8) -> Option<usize> {
        self.from_with = true;
        let done = self.with_proto_at(p, with);
        self.from_with = false;
        done
    }

    fn with_proto_at(&mut self, p: usize, with: u8) -> Option<usize> {
        let c = self.at(p);
        let n = self.b.len();
        match with {
            W_RECIPE => {
                if p >= n
                    && let Some(e) = self.pop(p, p, PLAIN)
                {
                    return Some(e);
                }
                if let Some(e) = self.proto(p) {
                    return Some(e);
                }
                self.substitution_at(p)
            }
            W_CALL => {
                if c == b')'
                    && p < n
                    && let Some(e) = self.pop(p, p, PLAIN)
                {
                    return Some(e);
                }
                if let Some(e) = self.substitution_at(p) {
                    return Some(e);
                }
                match c {
                    b'\'' if p < n => self.push(p, p + 1, mk::STR_BEGIN, &[STRING_SINGLE]),
                    b'"' if p < n => self.push(p, p + 1, mk::STR_BEGIN, &[STRING_DOUBLE]),
                    b'(' if p < n => self.push(p, p + 1, PLAIN, &[PAREN_BALANCER]),
                    _ => None,
                }
            }
            W_ASSIGN => {
                if let Some(e) = self.substitution_at(p) {
                    return Some(e);
                }
                if p >= n
                    && let Some(e) = self.pop(p, p, PLAIN)
                {
                    return Some(e);
                }
                if c == b')' && p < n {
                    return self.pop(p, p + 1, PLAIN);
                }
                None
            }
            _ => {
                let q = match with {
                    W_EMBED_SQ => b'\'',
                    W_EMBED_DQ => b'"',
                    _ => b'`',
                };
                if c == q && p < n {
                    return self.pop(p, p, PLAIN);
                }
                None
            }
        }
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

    /// The first place from `from` where a make pattern of embedding `with` may start.
    #[inline(never)]
    pub(super) fn next_with(&mut self, from: usize, with: u8) -> usize {
        let (w, f, at) = self.with_next;
        if w == with && f <= from && from <= at {
            return at;
        }
        let at = self.find_with(from, with);
        self.with_next = (with, from, at);
        at
    }

    fn find_with(&self, from: usize, with: u8) -> usize {
        let n = self.b.len();
        // `$` pops a recipe's and an assignment's contexts at the line's end
        let end = if matches!(with, W_RECIPE | W_ASSIGN) {
            from.max(n)
        } else {
            usize::MAX
        };
        if from >= n {
            return end;
        }
        let bit = 1u8 << with;
        let Some(i) = self.b[from..]
            .iter()
            .position(|&c| TRIGGER[usize::from(c)] & bit != 0)
        else {
            return end;
        };
        let mut i = from + i;
        // a comment starts at the spaces before its `#`
        if self.b[i] == b'#' {
            while i > from && super::ws(self.b[i - 1]) {
                i -= 1;
            }
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    #[test]
    fn a_rule_and_its_recipe() {
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
        assert!(m.contains(&("grep", "meta.function-call.shell variable.function.shell")));
        assert!(m.contains(&(" check", "comment.line.number-sign.shell")));
    }

    #[test]
    fn shell_assignments_and_calls() {
        let m = marked(
            Lang::Makefile,
            &[
                "DATE != date +%F",
                "REV := $(shell git rev-parse HEAD)",
                "X = $(subst a,b,$(Y))",
            ],
        );
        assert!(m.contains(&("!=", "keyword.operator.assignment.makefile")));
        assert!(m.contains(&("date", "meta.function-call.shell variable.function.shell")));
        assert!(m.contains(&("shell", "support.function.builtin.makefile")));
        assert!(m.contains(&("git", "meta.function-call.shell variable.function.shell")));
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
}
