//! bash's scanner, of ada's own (X-093, ADR-40, ADR-41), and the shell Makefiles embed
//! ([`makefile`]: recipes, `!=`, `$(shell …)`, `bash -c '…'` on this scan, X-094).
//!
//! bat's `Bourne Again Shell (bash)` is the reference for scopes and the parity oracle, not the
//! design. **A line starts on the fast loop** ([`Scan::fast`], the floor probe's shape): with only
//! holders open, a command's words, options, parameters, strings closed on the line, operators,
//! assignments and declarations are read a token a turn, the command's position in a local;
//! whatever else it meets it hands to the stack loop at that token, the contexts the stack loop
//! would have there pushed, so its spans are the stack loop's exactly. A line it takes whole
//! leaves the state as it was.
//!
//! **The stack loop** is one loop over a line's bytes on a small stack of contexts of its own: the
//! *holders*, where a command may start (the script, `$(…)`, backticks, `(…)`, `{ … }`, a
//! function's body, a case clause's commands, `<(…)`); a command's *line contexts* (its name,
//! its arguments, a builtin's arguments with that builtin's options, an assignment's name and
//! value, `[ … ]`, `let`), which a line's end pops unless a `\` continues it; and the frames that
//! nest (strings, `${…}`, arithmetic, `[[ … ]]`, a case's patterns, arrays, here-documents). A
//! command's first word is read once and looked up once; plain text is passed a run at a time.
//!
//! **Kinds are leaves**, bash's (`super::k`): a byte's rule is its deepest non-`meta` scope's, so
//! text no token takes is the innermost string's (`x=$(cmd arg)`'s `arg` is `string.unquoted`),
//! an option's, a command name's or plain; each frame keeps that fill. Inside an option a
//! `variable.*` kind takes its twin ([`TWIN`]). **A command's name** is `support.function`, the
//! builtins' color, apart from its arguments (the user's ask, as zsh-syntax-highlighting colors
//! it), where bat's `variable.function` is every built-in theme's plain foreground.
//!
//! Between lines the stack is kept in [`State`]'s 28 levels, a byte a context; a builtin's
//! arguments carry the builtin's option set in their id, and a here-document its end word as a
//! [`Label`] (X-105's primitive, 28 bits) in the four levels under it (bytes with the top bit set,
//! which no context has). Where one line opens several, the later opener's levels go under the
//! earlier's, so the bodies follow in the openers' order (bat reads the last opened first).

use std::mem::MaybeUninit;

use crate::emit::{Rule, Rules, Run, Runs, Sink, Spans};
use crate::heredoc::{Label, Lead, Tail};
use crate::prim::{char_len, find_in};
use crate::{Out, State};

mod makefile;

pub(crate) use makefile::RunLine as MakeRunLine;
pub use makefile::{line as make_line, scopes as make_scopes};

macro_rules! kinds {
    ($($k:ident = $s:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(u16)]
        enum KindId { $($k,)* Count }
        /// The kinds, each a stack of scopes under `source.shell.bash`.
        #[allow(dead_code)]
        mod k {
            $(pub const $k: u16 = super::KindId::$k as u16;)*
        }
        static KINDS: [&str; KindId::Count as usize] = [$($s,)*];
    };
}

kinds! {
    COMMENT = "comment.line.number-sign.shell",
    COMMENT_BEGIN = "comment.line.number-sign.shell punctuation.definition.comment.begin.shell",
    CONTINUATION = "punctuation.separator.continuation.line.shell",
    EXTRA_SPACES = "invalid.illegal.extraneous-spaces-after-line-continuation.shell",
    ESCAPE = "constant.character.escape.shell",
    FUNCTION_CALL = "meta.function-call.shell variable.function.shell",
    STR_DOUBLE = "string.quoted.double.shell",
    STR_SINGLE = "string.quoted.single.shell",
    STR_ANSI_C = "string.quoted.single.ansi-c.shell",
    STR_LOCALE = "string.quoted.double.locale.shell",
    STR_BEGIN = "punctuation.definition.string.begin.shell",
    STR_END = "punctuation.definition.string.end.shell",
    UNQUOTED = "string.unquoted.shell",
    OPTION = "variable.parameter.option.shell",
    OPTION_MARK = "punctuation.definition.parameter.shell",
    OPTION_ASSIGN = "keyword.operator.assignment.option.shell",
    END_OF_OPTIONS = "keyword.operator.end-of-options.shell",
    CONTINUE = "keyword.operator.logical.continue.shell",
    OR = "keyword.operator.logical.or.shell",
    PIPE = "keyword.operator.logical.pipe.shell",
    AND = "keyword.operator.logical.and.shell",
    JOB = "keyword.operator.logical.job.shell",
    LOGICAL = "keyword.operator.logical.shell",
    LOGICAL_NOT = "keyword.operator.logical.not.shell",
    COMPOUND_BEGIN = "punctuation.definition.compound.begin.shell",
    COMPOUND_END = "punctuation.definition.compound.end.shell",
    BRACES_BEGIN = "punctuation.definition.compound.braces.begin.shell",
    BRACES_END = "punctuation.definition.compound.braces.end.shell",
    DOUBLE_BRACE_BEGIN = "support.function.double-brace.begin.shell",
    DOUBLE_BRACE_END = "support.function.double-brace.end.shell",
    TEST_BEGIN = "support.function.test.begin.shell",
    TEST_END = "support.function.test.end.shell",
    LET = "support.function.let.bash",
    IF = "keyword.control.conditional.if.shell",
    THEN = "keyword.control.conditional.then.shell",
    ELIF = "keyword.control.conditional.elseif.shell",
    COND_END = "keyword.control.conditional.end.shell",
    ELSE = "keyword.control.conditional.else.shell",
    FOR = "keyword.control.loop.for.shell",
    DO = "keyword.control.loop.do.shell",
    DONE = "keyword.control.loop.end.shell",
    WHILE = "keyword.control.loop.while.shell",
    UNTIL = "keyword.control.loop.until.shell",
    CASE = "keyword.control.conditional.case.shell",
    CONTINUE_KW = "keyword.control.flow.continue.shell",
    BREAK = "keyword.control.flow.break.shell",
    IN = "keyword.control.in.shell",
    PATTERNS_BEGIN = "keyword.control.conditional.patterns.begin.shell",
    PATTERNS_END = "keyword.control.conditional.patterns.end.shell",
    CASE_END = "meta.conditional.case.clause.commands.shell punctuation.terminator.case.clause.shell",
    PARENS_BEGIN = "punctuation.section.parens.begin.shell",
    PARENS_END = "punctuation.section.parens.end.shell",
    BRACKETS_BEGIN = "punctuation.section.brackets.begin.shell",
    BRACKETS_END = "punctuation.section.brackets.end.shell",
    SECTION_BRACES_BEGIN = "punctuation.section.braces.begin.shell",
    SECTION_BRACES_END = "punctuation.section.braces.end.shell",
    FUNCTION_KW = "storage.type.function.shell",
    FUNCTION_NAME = "entity.name.function.shell",
    ARRAY_VAR = "variable.language.array.shell",
    ARITHMETIC_BEGIN = "punctuation.section.arithmetic.begin.shell",
    ARITHMETIC_END = "punctuation.section.arithmetic.end.shell",
    MODIFIER = "storage.modifier.shell",
    ALIAS = "support.function.alias.shell",
    ALIAS_NAME = "entity.name.function.alias.shell",
    VAR_ASSIGN = "variable.other.readwrite.assignment.shell",
    ASSIGN = "keyword.operator.assignment.shell",
    FD = "constant.numeric.integer.decimal.file-descriptor.shell",
    REDIRECT = "keyword.operator.assignment.redirection.shell",
    REDIRECT_PROCESS = "keyword.operator.assignment.redirection.process.shell",
    FD_END = "punctuation.terminator.file-descriptor.shell",
    HERE_STRING = "keyword.operator.herestring.shell",
    HISTORY = "variable.language.history.shell",
    HISTORY_MARK = "punctuation.definition.history.shell",
    GROUP_BEGIN = "punctuation.section.group.begin.shell",
    GROUP_END = "punctuation.section.group.end.shell",
    VAR_MARK = "punctuation.definition.variable.shell",
    PARAMETER_BEGIN = "punctuation.section.expansion.parameter.begin.shell",
    PARAMETER_END = "meta.group.expansion.parameter.shell punctuation.section.expansion.parameter.end.shell",
    READWRITE = "variable.other.readwrite.shell",
    LANG_VAR = "variable.language.shell",
    INDIRECTION = "keyword.operator.indirection.shell",
    ARITH_OP = "keyword.operator.arithmetic.shell",
    EXPANSION_OP = "keyword.operator.expansion.shell",
    QUANTIFIER = "keyword.operator.regexp.quantifier.shell",
    SET_BEGIN = "keyword.control.regexp.set.begin.shell",
    SET_END = "keyword.control.regexp.set.end.shell",
    WORD_OP = "keyword.operator.word.shell",
    CLASS_BEGIN = "punctuation.separator.character-class.begin.shell",
    CLASS_END = "punctuation.separator.character-class.end.shell",
    COLLATE_BEGIN = "punctuation.separator.collate.begin.shell",
    COLLATE_END = "punctuation.separator.collate.end.shell",
    EQUIVALENCE_BEGIN = "punctuation.separator.equivalence-class.begin.shell",
    EQUIVALENCE_END = "punctuation.separator.equivalence-class.end.shell",
    HEREDOC_TOKEN = "keyword.control.heredoc-token.shell",
    HEREDOC = "string.unquoted.heredoc.shell",
    HEREDOC_SPACES = "invalid.illegal.no-spaces-allowed-after-heredoc-token.shell",
    BACKTICK_ESCAPE = "constant.character.escape.backtick.shell",
    STRING_MARK = "punctuation.definition.string.shell",
    SUBSTITUTION = "keyword.operator.substitution.shell",
    SWITCH = "variable.parameter.switch.shell",
    SUBSTRING_BEGIN = "keyword.operator.substring.begin.shell",
    SUBSTRING_END = "keyword.operator.substring.end.shell",
    TILDE = "meta.group.expansion.tilde variable.language.tilde.shell",
    BRACE_BEGIN = "punctuation.section.expansion.brace.begin.shell",
    BRACE_END = "punctuation.section.expansion.brace.end.shell",
    SEPARATOR = "punctuation.separator.shell",
    JOB_MARK = "meta.group.expansion.job.shell punctuation.definition.variable.job.shell",
    JOB_VAR = "variable.language.job.shell",
    JOB_NUMBER = "constant.numeric.integer.decimal.job.shell",
    DECIMAL = "constant.numeric.integer.decimal.shell",
    HEX = "constant.numeric.integer.hexadecimal.shell",
    OCTAL = "constant.numeric.integer.octal.shell",
    OTHER_BASE = "constant.numeric.integer.other.shell",
    // a base's mark under its number's scope, as syntect stacks it (the pushed context's meta
    // scope over the match), so a theme's `constant.numeric punctuation` reaches it (HL-005)
    HEX_BASE = "constant.numeric.integer.hexadecimal.shell punctuation.definition.numeric.base.shell",
    OCTAL_BASE = "constant.numeric.integer.octal.shell punctuation.definition.numeric.base.shell",
    OTHER_BASE_MARK = "constant.numeric.integer.other.shell punctuation.definition.numeric.base.shell",
    NOT_HEX = "invalid.illegal.not-a-hex-character.shell",
    NOT_OCTAL = "invalid.illegal.not-an-octal-character.shell",
    BITWISE = "keyword.operator.bitwise.shell",
    TERNARY = "keyword.operator.ternary.shell",
    COPROC = "keyword.other.coproc.shell",
    COPROC_NAME = "entity.name.function.coproc.shell",
    // the builtins, in [`BUILTINS`]' order
    B_SET = "meta.function-call.shell support.function.set.shell",
    B_HELP = "meta.function-call.shell support.function.help.shell",
    B_MAPFILE = "meta.function-call.shell support.function.mapfile.shell",
    B_GETOPTS = "meta.function-call.shell support.function.getopts.shell",
    B_CD = "meta.function-call.shell support.function.cd.shell",
    B_SUSPEND = "meta.function-call.shell support.function.suspend.shell",
    B_COLON = "meta.function-call.shell support.function.colon.shell",
    B_PRINTF = "meta.function-call.shell support.function.printf.shell",
    B_HASH = "meta.function-call.shell support.function.hash.shell",
    B_ULIMIT = "meta.function-call.shell support.function.ulimit.shell",
    B_UMASK = "meta.function-call.shell support.function.umask.shell",
    B_SOURCE = "meta.function-call.shell support.function.source.shell",
    B_BUILTIN = "meta.function-call.shell support.function.builtin.shell",
    B_EXIT = "meta.function-call.shell support.function.exit.shell",
    B_TYPE = "meta.function-call.shell support.function.type.shell",
    B_BG = "meta.function-call.shell support.function.bg.shell",
    B_RETURN = "meta.function-call.shell keyword.control.flow.return.shell",
    B_EXEC = "meta.function-call.shell support.function.exec.shell",
    B_READ = "meta.function-call.shell support.function.read.shell",
    B_ENABLE = "meta.function-call.shell support.function.enable.shell",
    B_PWD = "meta.function-call.shell support.function.pwd.shell",
    B_LOGOUT = "meta.function-call.shell support.function.logout.shell",
    B_EVAL = "meta.function-call.shell support.function.eval.shell",
    B_FG = "meta.function-call.shell support.function.fg.shell",
    B_DISOWN = "meta.function-call.shell support.function.disown.shell",
    B_ECHO = "meta.function-call.shell support.function.echo.shell",
    B_DOT = "meta.function-call.shell support.function.dot.shell",
    B_WAIT = "meta.function-call.shell support.function.wait.shell",
    B_JOBS = "meta.function-call.shell support.function.jobs.shell",
    B_BIND = "meta.function-call.shell support.function.bind.shell",
    B_CALLER = "meta.function-call.shell support.function.caller.shell",
    B_TIMES = "meta.function-call.shell support.function.times.shell",
    B_UNALIAS = "meta.function-call.shell support.function.unalias.shell",
    B_COMMAND = "meta.function-call.shell support.function.command.shell",
    B_TRAP = "meta.function-call.shell support.function.trap.shell",
    B_SHIFT = "meta.function-call.shell support.function.shift.shell",
    B_KILL = "meta.function-call.shell support.function.kill.shell",
    B_READARRAY = "meta.function-call.shell support.function.readarray.shell",
    B_SHOPT = "meta.function-call.shell support.function.shopt.shell",
    B_UNSET = "meta.function-call.shell support.function.unset.shell",
    // a command's name in the builtins' color, apart from its arguments (X-093, the user's ask:
    // "similar to the zsh syntax highlighting plugin"); bat's `variable.function` is every
    // theme's plain foreground
    COMMAND = "meta.function-call.shell support.function.command.shell",
}

/// The kinds a host's here-documents take from bash's (Dockerfile's, X-105): the opener's
/// operator, its word's quotes and the word, a body's text.
pub(crate) mod hd {
    pub(crate) use super::k::{HEREDOC, HEREDOC_TOKEN, REDIRECT, STR_BEGIN, STR_END};
}

/// A `variable.*` kind inside an option is this plus the kind: it carries the option's scope
/// too, whose `variable.parameter` selector then outranks `variable` (the module's doc).
pub const TWIN: u16 = 0x200;

/// The scopes under `source.shell.bash` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let (outer, kind) = if kind >= TWIN {
        (Some("variable.parameter.option.shell"), kind - TWIN)
    } else {
        (None, kind)
    };
    KINDS.get(usize::from(kind)).map_or_else(Vec::new, |s| {
        outer.into_iter().chain(s.split(' ')).collect()
    })
}

/// Whether a kind's deepest scope is a `variable.*` one an option's scope outranks.
fn is_variable(kind: u16) -> bool {
    matches!(
        kind,
        k::FUNCTION_CALL
            | k::ARRAY_VAR
            | k::VAR_ASSIGN
            | k::HISTORY
            | k::READWRITE
            | k::LANG_VAR
            | k::SWITCH
            | k::TILDE
            | k::JOB_VAR
    ) || makefile::is_variable(kind)
}

// --- the contexts ---------------------------------------------------------------------------

// holders: a command may start where one is on top
const ROOT: u8 = 0;
/// `$(…)`: `)` closes it.
const EXPCMD: u8 = 1;
/// `` `…` ``: a backtick closes it, and the line contexts in it ([`BT`]).
const TICK: u8 = 2;
/// `(…)`: `)` closes it, arguments after.
const SUBSHELL: u8 = 3;
/// `{ …; }`: `}` closes it, arguments after.
const BRACES: u8 = 4;
/// A function's body: `}` or `)` closes it.
const FUNCBODY: u8 = 5;
/// A case clause's commands: `;;`, `;;&` or `;&` close them, `esac` ahead ends them.
const CASECMDS: u8 = 6;
/// `<(…)`, `>(…)`.
const PROC: u8 = 7;
/// A here-document's first line after its word (bat's `main-with-pop-at-end`): the line's end
/// pops it.
const LINEMAIN: u8 = 8;
/// bat's `cmd` without `main` around it: a command's name, keyword or builtin, but no
/// assignment, redirection or definition (after `while`, `until`, `continue`; after `(…)` and
/// `{ …; }` and an operator on their line). The line's end pops it.
const CMDONLY: u8 = 9;
const LAST_HOLDER: u8 = CMDONLY;

// a command's line contexts
/// A command's name.
const NAME: u8 = 10;
/// A command's arguments, with options.
const ARGS: u8 = 11;
/// After `--`: no options.
const EOO: u8 = 12;
/// An option's name after its `-`.
const OPTION: u8 = 13;
/// After `]]`, `]`: an operator, else a command.
const POST: u8 = 14;
/// After an assignment: more assignments (bat's `vardef-maybe-more`).
const MORE: u8 = 15;
/// An assignment's name.
const VARNAME: u8 = 16;
/// An assignment's unquoted value.
const VALUE: u8 = 17;
/// `alias`'s name.
const ALIASNAME: u8 = 18;
/// `declare`'s, `typeset`'s and `local`'s options; `export`'s; `readonly`'s; `alias`'s.
const DECL: u8 = 19;
const DECL_EXPORT: u8 = 20;
const DECL_READONLY: u8 = 21;
const DECL_ALIAS: u8 = 22;
/// `for`'s and `select`'s words.
const FOR: u8 = 23;
/// `[ … ]`.
const TEST: u8 = 24;
/// `let`'s expression, to the line's end.
const LET: u8 = 25;
/// A redirection's target.
const TARGET: u8 = 26;
/// `function`'s name.
const FUNCNAME: u8 = 27;
const LAST_LINE: u8 = FUNCNAME;

// frames that outlive a line
/// Between a function's name and its body's `{` or `(`.
const FUNCPARENS: u8 = 28;
/// `x=(…)`.
const ARRAYVAL: u8 = 29;
/// `[…]` in `x=(…)`.
const ARRAYIDX: u8 = 30;
/// `case`'s word, up to `in`.
const CASEWORD: u8 = 31;
/// A case's clauses.
const CASEBODY: u8 = 32;
/// A clause's patterns, up to `)`.
const CASEPAT: u8 = 33;
/// `(…)` in a clause's patterns.
const PATPARENS: u8 = 34;
/// `[[ … ]]`.
const DTEST: u8 = 35;
/// `=~`'s pattern in `[[ … ]]`, up to a blank.
const REGEX: u8 = 36;
/// `(( … ))`.
const ARITH: u8 = 37;
/// `$(( … ))`.
const XARITH: u8 = 38;
/// `(…)` in an expression.
const PARENS: u8 = 39;
/// `${` past its first character: the name.
const PARAM: u8 = 40;
/// `${…}` past an operator.
const PARAMC: u8 = 41;
/// `${x/…}`'s pattern.
const PARAMPAT: u8 = 42;
/// `${x##…}`'s pattern.
const PARAMX: u8 = 43;
/// `${x:…}`'s offset, and its length after the second `:`.
const SUBSTR: u8 = 44;
const SUBSTR2: u8 = 45;
/// `[…]` in `${x[…]}` or an assignment's name.
const INDEX: u8 = 46;
/// `?(…)`, `*(…)`, `+(…)`, `@(…)`, `!(…)`.
const EXTGLOB: u8 = 47;
/// `[…]` in a pattern.
const SET: u8 = 48;
/// `{a,b}`.
const BRACEX: u8 = 49;
const DQ: u8 = 50;
const SQ: u8 = 51;
/// `$'…'`.
const ANSI: u8 = 52;
/// `$"…"`.
const LOCALE: u8 = 53;
/// A here-document's body: `HEREDOC` plus [`HD_TABS`] (`<<-`) and [`HD_RAW`] (a quoted word: no
/// expansions).
const HEREDOC: u8 = 56;
const HD_TABS: u8 = 1;
const HD_RAW: u8 = 2;
/// A builtin's arguments, its option set ([`OPTS`]) added.
const BARGS: u8 = 64;

/// A level that holds 7 bits of a here-document's [`Label`].
const DATA: u8 = 0x80;

/// The contexts a line's end pops (unless a `\` continues the line).
const fn line_ends(c: u8) -> bool {
    (c > LAST_HOLDER && c <= LAST_LINE)
        || c == LINEMAIN
        || c == CMDONLY
        || c == REGEX
        || c >= BARGS && c < DATA
}

/// The contexts whose backtick ends them (bat's `-bt` contexts), when a backtick holder is
/// their nearest one.
const fn keeps_bt(c: u8) -> bool {
    matches!(
        c,
        FUNCBODY | BRACES | CMDONLY | NAME | ARGS | EOO | OPTION | POST | TEST | LET | TARGET
    ) || c >= BARGS && c < DATA
}

/// Each context's traits, a load: its own fill in the low 16 bits ([`own_fill`]),
/// [`I_LINE_ENDS`], [`I_KEEPS_BT`].
static INFO: [u32; 128] = {
    let mut t = [0u32; 128];
    let mut c = 0;
    while c < 128 {
        let x = c as u8;
        let mut v = own_fill(x) as u32;
        if line_ends(x) {
            v |= I_LINE_ENDS;
        }
        if keeps_bt(x) {
            v |= I_KEEPS_BT;
        }
        t[c] = v;
        c += 1;
    }
    t
};
const I_LINE_ENDS: u32 = 1 << 16;
const I_KEEPS_BT: u32 = 1 << 17;

/// Whether a line's end pops `c` ([`line_ends`], from [`INFO`]); a here-document's levels
/// never.
#[inline(always)]
fn ends_line(c: u8) -> bool {
    c < DATA && INFO[usize::from(c)] & I_LINE_ENDS != 0
}

/// A frame's own fill, if it has one (0: it takes the one below).
const fn own_fill(c: u8) -> u16 {
    match c {
        NAME => COMMAND,
        OPTION => k::OPTION,
        VARNAME => k::VAR_ASSIGN,
        VALUE => k::UNQUOTED,
        ALIASNAME => k::ALIAS_NAME,
        FUNCNAME => k::FUNCTION_NAME,
        PARAM => k::READWRITE,
        DQ => k::STR_DOUBLE,
        SQ => k::STR_SINGLE,
        ANSI => k::STR_ANSI_C,
        LOCALE => k::STR_LOCALE,
        _ if c & !3 == HEREDOC => k::HEREDOC,
        _ => 0,
    }
}

// frame flags
/// In a backtick context ([`keeps_bt`]).
const BT: u8 = 1;
/// Inside an option: `variable.*` kinds take their twins.
const TW: u8 = 2;

/// A command's name: [`k::COMMAND`], or bat's `variable.function` where ada-parse is built with
/// `ADA_PARSE_X093_BAT_CMD` set, for parity runs that measure the scan apart from that departure.
const COMMAND: u16 = if option_env!("ADA_PARSE_X093_BAT_CMD").is_some() {
    k::FUNCTION_CALL
} else {
    k::COMMAND
};

/// No kind: plain text.
const PLAIN: u16 = 0xffff;

// --- byte classes ---------------------------------------------------------------------------

/// `[A-Za-z0-9_]`, and bytes past ASCII (Oniguruma's `\w` is Unicode's).
static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
        c += 1;
    }
    t
};

/// A metacharacter: a blank, `|&;()<>`.
static META: [bool; 256] = {
    let mut t = [false; 256];
    let s = b" \t\r\n|&;()<>";
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
};

/// What an argument's plain run stops at: blanks, the metacharacters, quotes, `$`, backticks,
/// `\`, the patterns' and expansions' first bytes (`*?[{}~%`), `=` (an option's) and `]`.
static ARG_STOP: [bool; 256] = stops(b" \t\r|&;()<>\"'$`\\*?[{}~%]=");
/// A command name's plain run: `` [`=|&;()<>\s] ``, `}`, quotes, `$`, `~`, `%`, `\`.
static NAME_STOP: [bool; 256] = stops(b" \t\r|&;()<>`=}\"'$~%\\");
/// An unquoted value's: the metacharacters, backticks, quotes, `$`, `\`, the patterns'.
static VALUE_STOP: [bool; 256] = stops(b" \t\r|&;()<>`\"'$\\*?[{~%");
/// An expression's: operators, parentheses, quotes, `$`, backticks, `\`, `#`, digits' starts,
/// `]`, `:`, `}`.
static EXPR_STOP: [bool; 256] = {
    let mut t = stops(b" \t\r()\"'$`\\#]:}*/%+-&^|<>=!~,;?");
    let mut c = b'0';
    while c <= b'9' {
        t[c as usize] = true;
        c += 1;
    }
    t
};
/// `[[ … ]]`'s: blanks, `-=!<>|&`, quotes, `$`, backticks, `\`, the patterns'.
static TEST_STOP: [bool; 256] = stops(b" \t\r-=!<>|&\"'$`\\*?[{~%;()]");

const fn stops(s: &[u8]) -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < s.len() {
        t[s[i] as usize] = true;
        i += 1;
    }
    t
}

/// Oniguruma's `\s` on a line: a blank, `\r`, a form feed, a vertical tab.
#[inline(always)]
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// Whether `w` is at `i`.
#[inline(always)]
fn has(b: &[u8], i: usize, w: &[u8]) -> bool {
    b.len() >= i + w.len() && &b[i..i + w.len()] == w
}

/// Whether a word ends at `e`: bat's `keyword_break`, `(?![-=\w])`.
#[inline(always)]
fn breaks(b: &[u8], e: usize) -> bool {
    e >= b.len() || !(WORD[usize::from(b[e])] || b[e] == b'-' || b[e] == b'=')
}

/// Whether a builtin's name ends at `e`: `(?=\s|;|$|>|<|\|)`.
#[inline(always)]
fn builtin_ends(b: &[u8], e: usize) -> bool {
    e >= b.len() || is_space(b[e]) || matches!(b[e], b';' | b'>' | b'<' | b'|')
}

/// Whether `e` ends a builtin's option: `(?=\s|;|$|`|\))`.
#[inline(always)]
fn option_ends(b: &[u8], e: usize) -> bool {
    e >= b.len() || is_space(b[e]) || matches!(b[e], b';' | b'`' | b')')
}

// --- the command words ----------------------------------------------------------------------

// what a command's first word is
const W_NONE: u8 = 0;
const W_IF: u8 = 1; // a keyword a command follows: if then elif else do
const W_STAY: u8 = 14; // while until continue: bat's `cmd` goes on
const W_ARGS: u8 = 2; // fi done break: arguments follow
const W_FOR: u8 = 3;
const W_CASE: u8 = 4;
const W_ESAC: u8 = 5;
const W_FUNCTION: u8 = 6;
const W_COPROC: u8 = 7;
const W_DECLARE: u8 = 8; // declare typeset local
const W_EXPORT: u8 = 9;
const W_READONLY: u8 = 10;
const W_ALIAS: u8 = 11;
const W_LET: u8 = 12;
const W_BUILTIN: u8 = 13;

/// The command words of [`word`] past `:` and `.`, for [`WORDS`].
const WORD_LIST: [&[u8]; 61] = [
    b"if",
    b"then",
    b"elif",
    b"else",
    b"fi",
    b"for",
    b"do",
    b"done",
    b"while",
    b"until",
    b"case",
    b"continue",
    b"break",
    b"esac",
    b"function",
    b"coproc",
    b"declare",
    b"typeset",
    b"local",
    b"export",
    b"readonly",
    b"alias",
    b"let",
    b"set",
    b"help",
    b"mapfile",
    b"getopts",
    b"cd",
    b"suspend",
    b"printf",
    b"hash",
    b"ulimit",
    b"umask",
    b"source",
    b"builtin",
    b"exit",
    b"type",
    b"bg",
    b"return",
    b"exec",
    b"read",
    b"enable",
    b"pwd",
    b"logout",
    b"eval",
    b"fg",
    b"disown",
    b"echo",
    b"wait",
    b"jobs",
    b"bind",
    b"caller",
    b"times",
    b"unalias",
    b"command",
    b"trap",
    b"shift",
    b"kill",
    b"readarray",
    b"shopt",
    b"unset",
];

/// A command word's row: its bytes packed little-endian (at most 8 of them), its length, and
/// [`word`]'s answer.
#[derive(Clone, Copy)]
struct Row {
    w: u64,
    len: u8,
    what: u8,
    o: u8,
    kind: u16,
}

/// The command words hashed by their packed bytes and length ([`slot`]), probed linearly: a
/// word is one load and one compare where [`word`]'s `match` called `memcmp` (`readarray`'s
/// nine bytes are its own test).
static WORDS: [Row; 256] = {
    let mut t = [Row {
        w: 0,
        len: 0,
        what: 0,
        o: 0,
        kind: 0,
    }; 256];
    let mut i = 0;
    while i < WORD_LIST.len() {
        let w = WORD_LIST[i];
        if w.len() <= 8 {
            let (what, kind, o) = word(w);
            let v = pack(w);
            let mut j = slot(v, w.len());
            while t[j].len != 0 {
                j = (j + 1) & 255;
            }
            t[j] = Row {
                w: v,
                len: w.len() as u8,
                what,
                o,
                kind,
            };
        }
        i += 1;
    }
    t
};

const fn pack(w: &[u8]) -> u64 {
    let mut v = 0u64;
    let mut i = 0;
    while i < w.len() && i < 8 {
        v |= (w[i] as u64) << (8 * i);
        i += 1;
    }
    v
}

#[inline(always)]
const fn slot(w: u64, len: usize) -> usize {
    ((w ^ len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 56) as usize
}

/// `b[p..e]`'s answer from [`WORDS`] ([`word`]'s).
#[inline(always)]
fn lookup(b: &[u8], p: usize, e: usize) -> (u8, u16, u8) {
    let len = e - p;
    if len > 8 {
        return if len == 9 && &b[p..e] == b"readarray" {
            word(b"readarray")
        } else {
            (W_NONE, 0, 0)
        };
    }
    let v = if p + 8 <= b.len() {
        let w = u64::from_le_bytes(b[p..p + 8].try_into().unwrap_or([0; 8]));
        w & (u64::MAX >> (64 - 8 * len))
    } else {
        pack(&b[p..e])
    };
    let mut j = slot(v, len);
    loop {
        let t = WORDS[j];
        if t.len == 0 {
            return (W_NONE, 0, 0);
        }
        if t.w == v && usize::from(t.len) == len {
            return (t.what, t.kind, t.o);
        }
        j = (j + 1) & 255;
    }
}

/// A command's first word: what it is, its kind, and a builtin's option set.
#[inline(always)]
const fn word(w: &[u8]) -> (u8, u16, u8) {
    use k::*;
    let (a, kind, o) = match w {
        b"if" => (W_IF, IF, 0),
        b"then" => (W_IF, THEN, 0),
        b"elif" => (W_IF, ELIF, 0),
        b"else" => (W_IF, ELSE, 0),
        b"fi" => (W_ARGS, COND_END, 0),
        b"for" => (W_FOR, FOR, 0),
        b"do" => (W_IF, DO, 0),
        b"done" => (W_ARGS, DONE, 0),
        b"while" => (W_STAY, WHILE, 0),
        b"until" => (W_STAY, UNTIL, 0),
        b"case" => (W_CASE, CASE, 0),
        b"continue" => (W_STAY, CONTINUE_KW, 0),
        b"break" => (W_ARGS, BREAK, 0),
        b"esac" => (W_ESAC, COND_END, 0),
        b"function" => (W_FUNCTION, FUNCTION_KW, 0),
        b"coproc" => (W_COPROC, COPROC, 0),
        b"declare" | b"typeset" | b"local" => (W_DECLARE, MODIFIER, 0),
        b"export" => (W_EXPORT, MODIFIER, 0),
        b"readonly" => (W_READONLY, MODIFIER, 0),
        b"alias" => (W_ALIAS, ALIAS, 0),
        b"let" => (W_LET, LET, 0),
        b"set" => (W_BUILTIN, B_SET, O_SET),
        b"help" => (W_BUILTIN, B_HELP, O_HELP),
        b"mapfile" => (W_BUILTIN, B_MAPFILE, O_MAPFILE),
        b"getopts" => (W_BUILTIN, B_GETOPTS, O_NONE),
        b"cd" => (W_BUILTIN, B_CD, O_CD),
        b"suspend" => (W_BUILTIN, B_SUSPEND, O_SUSPEND),
        b":" => (W_BUILTIN, B_COLON, O_NONE),
        b"printf" => (W_BUILTIN, B_PRINTF, O_PRINTF),
        b"hash" => (W_BUILTIN, B_HASH, O_HASH),
        b"ulimit" => (W_BUILTIN, B_ULIMIT, O_ULIMIT),
        b"umask" => (W_BUILTIN, B_UMASK, O_UMASK),
        b"source" => (W_BUILTIN, B_SOURCE, O_NONE),
        b"builtin" => (W_BUILTIN, B_BUILTIN, O_NONE),
        b"exit" => (W_BUILTIN, B_EXIT, O_NONE),
        b"type" => (W_BUILTIN, B_TYPE, O_TYPE),
        b"bg" => (W_BUILTIN, B_BG, O_NONE),
        b"return" => (W_BUILTIN, B_RETURN, O_NONE),
        b"exec" => (W_BUILTIN, B_EXEC, O_EXEC),
        b"read" => (W_BUILTIN, B_READ, O_READ),
        b"enable" => (W_BUILTIN, B_ENABLE, O_ENABLE),
        b"pwd" => (W_BUILTIN, B_PWD, O_PWD),
        b"logout" => (W_BUILTIN, B_LOGOUT, O_NONE),
        b"eval" => (W_BUILTIN, B_EVAL, O_NONE),
        b"fg" => (W_BUILTIN, B_FG, O_NONE),
        b"disown" => (W_BUILTIN, B_DISOWN, O_DISOWN),
        b"echo" => (W_BUILTIN, B_ECHO, O_ECHO),
        b"." => (W_BUILTIN, B_DOT, O_NONE),
        b"wait" => (W_BUILTIN, B_WAIT, O_WAIT),
        b"jobs" => (W_BUILTIN, B_JOBS, O_JOBS),
        b"bind" => (W_BUILTIN, B_BIND, O_BIND),
        b"caller" => (W_BUILTIN, B_CALLER, O_NONE),
        b"times" => (W_BUILTIN, B_TIMES, O_NONE),
        b"unalias" => (W_BUILTIN, B_UNALIAS, O_UNALIAS),
        b"command" => (W_BUILTIN, B_COMMAND, O_COMMAND),
        b"trap" => (W_BUILTIN, B_TRAP, O_TRAP),
        b"shift" => (W_BUILTIN, B_SHIFT, O_NONE),
        b"kill" => (W_BUILTIN, B_KILL, O_KILL),
        b"readarray" => (W_BUILTIN, B_READARRAY, O_READ),
        b"shopt" => (W_BUILTIN, B_SHOPT, O_SHOPT),
        b"unset" => (W_BUILTIN, B_UNSET, O_UNSET),
        _ => (W_NONE, 0, 0),
    };
    (a, kind, o)
}

/// A builtin's options (bat's generated `commands-builtin-shell-bash`): `(?:\s+|^)((-)D|B+)`
/// followed by `\s`, `;`, `` ` ``, `)` or the line's end, `D` a character of `dash` (`D+` where
/// `many`), `B` of `bare` (no `-`); `set`'s mark may be `+` too, its `o` alone.
struct Opts {
    dash: u128,
    many: bool,
    bare: u128,
}

const fn set_of(s: &[u8]) -> u128 {
    let mut m = 0u128;
    let mut i = 0;
    while i < s.len() {
        m |= 1 << s[i];
        i += 1;
    }
    m
}

const fn opts(dash: &[u8], many: bool, bare: &[u8]) -> Opts {
    Opts {
        dash: set_of(dash),
        many,
        bare: set_of(bare),
    }
}

const O_NONE: u8 = 0;
const O_SET: u8 = 1;
const O_BIND: u8 = 2;
const O_CD: u8 = 3;
const O_COMMAND: u8 = 4;
const O_DISOWN: u8 = 5;
const O_ECHO: u8 = 6;
const O_ENABLE: u8 = 7;
const O_EXEC: u8 = 8;
const O_HASH: u8 = 9;
const O_HELP: u8 = 10;
const O_JOBS: u8 = 11;
const O_KILL: u8 = 12;
const O_MAPFILE: u8 = 13;
const O_PRINTF: u8 = 14;
const O_PWD: u8 = 15;
const O_READ: u8 = 16;
const O_SHOPT: u8 = 17;
const O_SUSPEND: u8 = 18;
const O_TRAP: u8 = 19;
const O_TYPE: u8 = 20;
const O_ULIMIT: u8 = 21;
const O_UMASK: u8 = 22;
const O_UNALIAS: u8 = 23;
const O_UNSET: u8 = 24;
const O_WAIT: u8 = 25;

static OPTS: [Opts; 26] = [
    opts(b"", false, b""),
    opts(b"abefhkmnptuvxBCEHPT", true, b""), // set: `o` alone, `+` too
    opts(b"mqurfx", false, b"lpsvPSVX"),
    opts(b"LPe@", false, b""),
    opts(b"pVv", true, b""),
    opts(b"h", false, b"ar"),
    opts(b"neE", true, b""),
    opts(b"af", false, b"dnps"),
    opts(b"a", false, b"cl"),
    opts(b"rpl", false, b"dt"),
    opts(b"dms", true, b""),
    opts(b"x", false, b"lnprs"),
    opts(b"snlL", false, b""),
    opts(b"dnOstuCc", false, b""),
    opts(b"v", false, b""),
    opts(b"LP", true, b""),
    opts(b"adeinNprstu", false, b"ers"),
    opts(b"o", false, b"pqsu"),
    opts(b"f", false, b""),
    opts(b"lp", true, b""),
    opts(b"afptP", true, b""),
    opts(b"HSabcdefiklmnpqrstuvxPT", true, b""),
    opts(b"pS", false, b""),
    opts(b"a", false, b""),
    opts(b"fnv", true, b""),
    opts(b"n", false, b""),
];

#[inline(always)]
fn in_set(m: u128, c: u8) -> bool {
    c < 128 && m & (1 << c) != 0
}

// --- the scan -------------------------------------------------------------------------------

/// How many levels a line can hold; a push past it drops the bottom one.
const MAX: usize = 64;

struct Scan<'b, 'c, K: Sink> {
    b: &'b [u8],
    stack: [u8; MAX],
    /// Each level's fill (the kind of the text no token takes, [`PLAIN`] for none) in the low
    /// 16 bits, its flags above: written by [`Scan::derive`] before they are read, so never
    /// set up.
    meta: [MaybeUninit<u32>; MAX],
    depth: usize,
    /// The top level's context, fill and flags.
    cur: u8,
    text: u16,
    fl: u8,
    /// Where the spans so far end: the text from here to a token is the top's fill.
    at: usize,
    /// Whether the line ends in a continuation.
    cont: bool,
    /// The line's last `]`, and where its last `))` starts, once looked for (`usize::MAX`:
    /// not yet; `0` where none starts past the line's start, as none is ever asked for at 0).
    last_bracket: usize,
    last_close: usize,
    /// The lowest level of the here-documents this line opened (`usize::MAX`: none yet): a later
    /// opener's levels go there.
    hd_at: usize,
    k: K,
    out: &'b mut Out<'c>,
}

impl<'b, 'c, K: Sink> Scan<'b, 'c, K> {
    /// A scan of `b`, built in place; the state is [`Scan::load`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [ROOT; MAX],
            meta: [const { MaybeUninit::uninit() }; MAX],
            depth: 1,
            cur: ROOT,
            text: PLAIN,
            fl: 0,
            at: 0,
            cont: false,
            last_bracket: usize::MAX,
            last_close: usize::MAX,
            hd_at: usize::MAX,
            k,
            out,
        }
    }

    /// The stack from `state`, each level's fill and flags made again.
    #[inline(always)]
    fn load(&mut self, state: &State) {
        if state.depth == 0 {
            // the commonest line: nothing open (the scan is fresh's)
            self.meta[0] = MaybeUninit::new(u32::from(PLAIN));
            return;
        }
        self.depth = state.load(&mut self.stack);
        self.derive_all();
    }

    /// The state the scan's last line left.
    #[inline(always)]
    fn keep(&self) -> State {
        if self.depth == 1 {
            return State::default();
        }
        State::keep(&self.stack, self.depth, 0)
    }

    /// Each level's fill and flags from the bottom up.
    #[inline(always)]
    fn derive_all(&mut self) {
        self.meta[0] = MaybeUninit::new(u32::from(PLAIN));
        for d in 1..self.depth {
            self.derive(d);
        }
        self.top_from(self.depth);
    }

    /// Level `d`'s fill and flags from the levels under it, written and returned.
    #[inline(always)]
    fn derive(&mut self, d: usize) -> u32 {
        let c = self.stack[d];
        if c & DATA != 0 {
            self.meta[d] = self.meta[d - 1];
            // SAFETY: levels below `d` are written before `d` is
            return unsafe { self.meta[d - 1].assume_init() };
        }
        // under a here-document's body, its first line's commands take what was under it (under
        // every body the line opened)
        let mut u = d - 1;
        if c == LINEMAIN && self.stack[u] & !3 == HEREDOC {
            while u > 0 && (self.stack[u] & DATA != 0 || self.stack[u] & !3 == HEREDOC) {
                u -= 1;
            }
        }
        // SAFETY: levels below `d` are written before `d` is
        let m = derived(unsafe { self.meta[u].assume_init() }, c);
        self.meta[d] = MaybeUninit::new(m);
        m
    }

    /// The top's fill and flags, as a level keeps them.
    #[inline(always)]
    fn top_meta(&self) -> u32 {
        u32::from(self.text) | u32::from(self.fl) << 16
    }

    /// `c` made the top at level `d`, its fill and flags `m`: the top's fields set from
    /// registers, not read back from the levels.
    #[inline(always)]
    fn enter(&mut self, d: usize, c: u8, m: u32) {
        self.depth = d + 1;
        self.cur = c;
        self.text = m as u16;
        self.fl = (m >> 16) as u8;
    }

    #[inline(always)]
    fn top_from(&mut self, depth: usize) {
        let d = depth - 1;
        self.cur = self.stack[d];
        // SAFETY: the top level is written
        let m = unsafe { self.meta[d].assume_init() };
        self.text = m as u16;
        self.fl = (m >> 16) as u8;
    }

    /// The line lexed from the stack it starts with, ended as a state ends it.
    /// Whether the line left the stack as it was: the fast loop took it whole.
    #[inline(always)]
    fn lex_line(&mut self) -> bool {
        self.at = 0;
        self.cont = false;
        self.last_bracket = usize::MAX;
        self.last_close = usize::MAX;
        self.hd_at = usize::MAX;
        let mut p = 0;
        if FALLS && !self.fast_ok() {
            eprintln!("SLOW {} {} {}", self.cur, self.depth, self.b.len());
        }
        if self.fast_ok() {
            match self.fast(0) {
                None => return true,
                Some(q) => p = q,
            }
        }
        self.run(p);
        let n = self.b.len();
        self.fill(n);
        if !self.cont {
            while ends_line(self.cur) && self.depth > 1 {
                self.depth -= 1;
                self.top_from(self.depth);
            }
        }
        if self.depth - 1 > crate::stack::HELD {
            crate::stack::hold(&mut self.stack, &mut self.depth);
            self.derive_all();
        }
        false
    }

    // --- the stack ---

    /// `c` pushed at `p`, the text before it filled.
    #[inline(always)]
    fn push(&mut self, p: usize, c: u8) {
        self.fill(p);
        if self.depth == MAX {
            self.full();
        }
        let d = self.depth;
        self.stack[d] = c;
        // the level below is the top: its fill and flags are the top's fields (but under a
        // here-document's first line, [`Scan::derive`]'s)
        let m = if c & DATA != 0 {
            self.top_meta()
        } else if c == LINEMAIN {
            self.derive(d)
        } else {
            derived(self.top_meta(), c)
        };
        self.meta[d] = MaybeUninit::new(m);
        self.enter(d, c, m);
    }

    #[cold]
    #[inline(never)]
    fn full(&mut self) {
        crate::stack::drop_bottom(&mut self.stack, &mut self.depth);
        self.derive_all();
    }

    /// The top popped at `p`, the text before it filled.
    #[inline(always)]
    fn pop(&mut self, p: usize) {
        self.fill(p);
        if self.depth > 1 {
            self.depth -= 1;
            self.top_from(self.depth);
        }
    }

    /// The top replaced by `c` at `p`.
    #[inline(always)]
    fn set(&mut self, p: usize, c: u8) {
        self.fill(p);
        if self.depth > 1 {
            self.depth -= 1;
        }
        let d = self.depth;
        self.stack[d] = c;
        let m = self.derive(d);
        self.enter(d, c, m);
    }

    /// The text up to `p` is the top's fill.
    #[inline(always)]
    fn fill(&mut self, p: usize) {
        if self.text == PLAIN {
            self.at = self.at.max(p);
        } else if p > self.at {
            self.k.merge_call(&mut self.out, self.at, p, self.text);
            self.at = p;
        }
    }

    /// Bytes `s..e` are `kind` (its twin in an option), after the text before them.
    #[inline(always)]
    fn put(&mut self, s: usize, e: usize, kind: u16) -> usize {
        self.fill(s);
        let kind = twin(kind, self.fl);
        self.k.merge_call(&mut self.out, s, e, kind);
        self.at = e;
        e
    }

    // --- the loop ---

    #[inline(always)]
    fn run(&mut self, mut p: usize) {
        let b = self.b;
        let n = b.len();
        if p == 0 && self.cur & !3 == HEREDOC {
            p = self.heredoc_end();
        }
        // turns that didn't move: past a bound, a character is passed (syntect's loop guard,
        // for whatever pops and pushes back at one place)
        let mut stuck = 0u32;
        while p < n {
            let q = p;
            p = match self.cur {
                ROOT..=LAST_HOLDER => self.holder(p),
                NAME => self.name(p),
                ARGS | EOO | MORE | FOR | TEST | POST => self.args(p),
                OPTION => self.option(p),
                VARNAME | ALIASNAME => self.varname(p),
                VALUE => self.value(p),
                DECL..=DECL_ALIAS => self.decl(p),
                LET | ARITH | XARITH | PARENS | INDEX | SUBSTR | SUBSTR2 => self.expr(p),
                TARGET => self.target(p),
                FUNCNAME => self.funcname(p),
                FUNCPARENS => self.funcparens(p),
                ARRAYVAL | ARRAYIDX => self.array(p),
                CASEWORD | CASEBODY | CASEPAT | PATPARENS => self.case(p),
                DTEST | REGEX => self.dtest(p),
                PARAM | PARAMC | PARAMPAT | PARAMX => self.param(p),
                EXTGLOB | SET | BRACEX => self.pattern(p),
                DQ | LOCALE => self.dq(p),
                SQ => self.sq(p),
                ANSI => self.ansi(p),
                c if c & !3 == HEREDOC => self.heredoc(p),
                _ => self.args(p),
            };
            if p == q {
                stuck += 1;
                if stuck > 64 {
                    p = self.stuck(p);
                    stuck = 0;
                }
            } else {
                stuck = 0;
            }
        }
    }

    /// A place the scan turned at without moving: its character passed as the top's text.
    #[cold]
    #[inline(never)]
    fn stuck(&mut self, p: usize) -> usize {
        let e = (p + char_len(self.b[p])).min(self.b.len());
        self.fill(e);
        e
    }

    // --- shared pieces ---

    /// The prototype's pieces at `p`: a comment (`#` at the line's start or after a blank), a
    /// continuation or an escape; `None` where none starts.
    #[inline(always)]
    fn proto(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        match b[p] {
            b'#' if p == 0 || is_space(b[p - 1]) => Some(self.comment(p)),
            b'\\' => Some(self.backslash(p)),
            _ => None,
        }
    }

    /// Whether a comment follows the blank run at `p`.
    #[inline(always)]
    fn comment_follows(&self, p: usize) -> bool {
        let b = self.b;
        let mut e = p + 1;
        while e < b.len() && is_space(b[e]) {
            e += 1;
        }
        e < b.len() && b[e] == b'#'
    }

    /// A comment after the blank run at `p`, the blanks the top's text (the prototype matches
    /// before the context's own patterns, so a name's or an option's blanks take its scope).
    #[inline(always)]
    fn comment_after(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let mut e = p + 1;
        while e < n && is_space(b[e]) {
            e += 1;
        }
        if e < n && b[e] == b'#' {
            return Some(self.comment(e));
        }
        None
    }

    fn comment(&mut self, p: usize) -> usize {
        let n = self.b.len();
        self.put(p, p + 1, k::COMMENT_BEGIN);
        self.put(p + 1, n, k::COMMENT)
    }

    /// `\` at `p`: a continuation at the line's end (spaces between marked), else an escape of
    /// the character after it.
    #[inline(always)]
    fn backslash(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if p + 1 == n {
            self.cont = true;
            return self.put(p, n, k::CONTINUATION);
        }
        let mut e = p + 1;
        while e < n && is_space(b[e]) {
            e += 1;
        }
        if e == n {
            self.cont = true;
            self.fill(p);
            self.at = p + 1;
            return self.put(p + 1, n, k::EXTRA_SPACES);
        }
        self.put(p, (p + 1 + char_len(b[p + 1])).min(n), k::ESCAPE)
    }

    /// Strings and expansions at `p` (`what`: [`X_STR`] … which of them); `None` where none
    /// starts.
    #[inline(always)]
    fn expand(&mut self, p: usize, what: u8) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        Some(match c {
            b'"' if what & X_STR != 0 => {
                self.put(p, p + 1, k::STR_BEGIN);
                self.push(p + 1, DQ);
                p + 1
            }
            b'\'' if what & X_STR != 0 => {
                self.put(p, p + 1, k::STR_BEGIN);
                self.push(p + 1, SQ);
                p + 1
            }
            b'$' => return self.dollar(p, what),
            b'`' => {
                if self.fl & BT != 0 {
                    return None;
                }
                self.put(p, p + 1, k::GROUP_BEGIN);
                self.push(p + 1, TICK);
                p + 1
            }
            b'*' | b'?' if what & X_PAT != 0 => {
                if b.get(p + 1) == Some(&b'(') {
                    self.extglob(p)
                } else {
                    self.put(p, p + 1, k::QUANTIFIER)
                }
            }
            b'+' | b'@' | b'!' if what & X_PAT != 0 && b.get(p + 1) == Some(&b'(') => {
                self.extglob(p)
            }
            b'[' if what & X_PAT != 0 && self.bracket_after(p) => self.set_open(p),
            b'{' if what & X_BRACE != 0 => {
                self.put(p, p + 1, k::BRACE_BEGIN);
                self.push(p + 1, BRACEX);
                p + 1
            }
            b'~' if what & X_TILDE != 0 => self.put(p, p + 1, k::TILDE),
            b'%' if what & X_JOB != 0 => self.job(p),
            _ => return None,
        })
        .map(|e| e.min(n))
    }

    fn extglob(&mut self, p: usize) -> usize {
        self.put(p, p + 1, k::QUANTIFIER);
        self.put(p + 1, p + 2, k::PARENS_BEGIN);
        self.push(p + 2, EXTGLOB);
        p + 2
    }

    /// Whether a `]` is on the line past `p`.
    #[inline(always)]
    fn bracket_after(&mut self, p: usize) -> bool {
        if self.last_bracket == usize::MAX {
            self.last_bracket = self.b.iter().rposition(|&c| c == b']').unwrap_or(0);
        }
        self.last_bracket > p
    }

    /// Whether a `))` starts on the line at or past `p`.
    #[inline(always)]
    fn close_from(&mut self, p: usize) -> bool {
        if self.last_close == usize::MAX {
            let b = self.b;
            self.last_close = (1..b.len())
                .rev()
                .find(|&i| b[i] == b')' && b[i - 1] == b')')
                .map_or(0, |i| i - 1);
        }
        self.last_close >= p && self.last_close != 0
    }

    /// A pattern's `[` at `p`: its first character (`!`, `^`), then the set.
    fn set_open(&mut self, p: usize) -> usize {
        let b = self.b;
        self.put(p, p + 1, k::SET_BEGIN);
        let mut q = p + 1;
        match b.get(q) {
            Some(b'!' | b'^') => {
                self.put(q, q + 1, k::LOGICAL_NOT);
                q += 1;
            }
            Some(b'-') => q += 1,
            _ => {}
        }
        self.push(q, SET);
        q
    }

    /// `%`: a job.
    fn job(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mark = self.put(p, p + 1, k::JOB_MARK);
        match b.get(p + 1) {
            Some(b'%' | b'+' | b'-') => self.put(mark, mark + 1, k::JOB_VAR),
            Some(c) if c.is_ascii_digit() => {
                let mut e = mark;
                while e < n && b[e].is_ascii_digit() {
                    e += 1;
                }
                self.put(mark, e, k::JOB_NUMBER)
            }
            Some(&c) => {
                let q = if c == b'?' { mark + 1 } else { mark };
                let mut e = q;
                while e < n && WORD[usize::from(b[e])] {
                    e += 1;
                }
                if e == q {
                    return mark;
                }
                if q > mark {
                    self.put(mark, q, k::QUANTIFIER);
                }
                self.put(q, e, k::READWRITE)
            }
            None => mark,
        }
    }

    /// `$` at `p`: a parameter, `${…}`, `$(…)`, `$((…))`, `$'…'`, `$"…"`; `None` for a `$`
    /// alone.
    #[inline(always)]
    fn dollar(&mut self, p: usize, what: u8) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let &c = b.get(p + 1)?;
        Some(match c {
            b'{' => {
                self.put(p, p + 1, k::VAR_MARK);
                self.put(p + 1, p + 2, k::PARAMETER_BEGIN);
                let mut q = p + 2;
                match b.get(q) {
                    Some(b'!') => q = self.put(q, q + 1, k::INDIRECTION),
                    Some(b'#') => q = self.put(q, q + 1, k::ARITH_OP),
                    _ => {}
                }
                self.push(q, PARAM);
                q
            }
            b'(' => {
                if b.get(p + 2) == Some(&b'(') && what & X_CMD_FIRST == 0 && self.close_from(p + 4)
                {
                    self.put(p, p + 1, k::VAR_MARK);
                    self.put(p + 1, p + 3, k::PARENS_BEGIN);
                    self.push(p + 3, XARITH);
                    p + 3
                } else {
                    self.put(p, p + 1, k::VAR_MARK);
                    self.put(p + 1, p + 2, k::PARENS_BEGIN);
                    self.push(p + 2, EXPCMD);
                    p + 2
                }
            }
            b'\'' if what & X_STR != 0 => {
                self.put(p, p + 2, k::STR_BEGIN);
                self.push(p + 2, ANSI);
                p + 2
            }
            b'"' if what & X_STR != 0 => {
                self.put(p, p + 2, k::STR_BEGIN);
                self.push(p + 2, LOCALE);
                p + 2
            }
            b'0'..=b'9' => {
                self.put(p, p + 1, k::VAR_MARK);
                self.put(p + 1, p + 2, k::READWRITE)
            }
            b'$' | b'#' | b'@' | b'!' | b'~' | b'*' | b'?' | b'_' | b'-'
                if p + 2 >= n || !WORD[usize::from(b[p + 2])] =>
            {
                self.put(p, p + 1, k::VAR_MARK);
                self.put(p + 1, p + 2, k::LANG_VAR)
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                let mut e = p + 2;
                while e < n && WORD[usize::from(b[e])] {
                    e += 1;
                }
                self.put(p, p + 1, k::VAR_MARK);
                self.put(p + 1, e, k::READWRITE)
            }
            _ => return None,
        })
    }

    /// A redirection at `p`, its file descriptor from `d` (`d..p` digits); `None` where none
    /// starts.
    fn redirect(&mut self, d: usize, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let at = |i: usize| b.get(i).copied().unwrap_or(b'\n');
        let fd = |s: &mut Self| {
            if p > d {
                s.put(d, p, k::FD);
            }
        };
        if c == b'<' {
            if at(p + 1) == b'<' {
                if at(p + 2) == b'<' && (p + 3 >= n || is_space(b[p + 3])) {
                    fd(self);
                    return Some(self.put(p, p + 3, k::HERE_STRING));
                }
                if let Some(e) = self.heredoc_open(d, p) {
                    return Some(e);
                }
            }
            if at(p + 1) == b'(' {
                fd(self);
                self.put(p, p + 1, k::REDIRECT_PROCESS);
                self.put(p + 1, p + 2, k::PARENS_BEGIN);
                self.push(p + 2, PROC);
                return Some(p + 2);
            }
            fd(self);
            let e = if at(p + 1) == b'&' { p + 2 } else { p + 1 };
            self.put(p, e, k::REDIRECT);
            return Some(self.redirect_post(e));
        }
        if c == b'>' && at(p + 1) == b'(' {
            fd(self);
            self.put(p, p + 1, k::REDIRECT_PROCESS);
            self.put(p + 1, p + 2, k::PARENS_BEGIN);
            self.push(p + 2, PROC);
            return Some(p + 2);
        }
        // `>>!?|>&?|&>|&?>(?:\||>)`
        let e = if c == b'>' {
            if at(p + 1) == b'>' {
                if at(p + 2) == b'!' { p + 3 } else { p + 2 }
            } else if at(p + 1) == b'&' {
                p + 2
            } else {
                p + 1
            }
        } else if c == b'&' && at(p + 1) == b'>' {
            p + 2
        } else {
            return None;
        };
        fd(self);
        self.put(p, e, k::REDIRECT);
        Some(self.redirect_post(e))
    }

    /// After a redirection's operator: a descriptor, `-`, or the target.
    fn redirect_post(&mut self, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = e;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        if q == n {
            return e;
        }
        if b[q].is_ascii_digit() {
            let mut r = q;
            while r < n && b[r].is_ascii_digit() {
                r += 1;
            }
            return self.put(q, r, k::FD);
        }
        if b[q] == b'-' {
            return self.put(q, q + 1, k::FD_END);
        }
        self.push(q, TARGET);
        q
    }

    /// `<<` at `p`: a here-document's word, when it is one, and its body pushed below the rest
    /// of the line.
    fn heredoc_open(&mut self, d: usize, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let mut q = p + 2;
        let tabs = b.get(q) == Some(&b'-');
        if tabs {
            q += 1;
        }
        let op = q;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        let quote = match b.get(q) {
            Some(&c @ (b'\'' | b'"' | b'\\')) => Some(c),
            _ => None,
        };
        let s = if quote.is_some() { q + 1 } else { q };
        if s >= n || !(b[s].is_ascii_alphabetic() || b[s] == b'_' || b[s] >= 0x80) {
            return None;
        }
        let mut e = s + 1;
        while e < n && WORD[usize::from(b[e])] {
            e += 1;
        }
        let end = match quote {
            Some(c @ (b'\'' | b'"')) => {
                if b.get(e) != Some(&c) {
                    return None;
                }
                e + 1
            }
            _ => e,
        };
        if p > d {
            self.put(d, p, k::FD);
        }
        self.put(p, op, k::REDIRECT);
        if q > op {
            // the blanks before the word, under the body's scope as its push's token
            self.put(op, q, k::HEREDOC);
        }
        match quote {
            Some(b'\\') => {
                self.put(q, s, k::STRING_MARK);
            }
            Some(_) => {
                self.put(q, s, k::STR_BEGIN);
            }
            None => {}
        }
        self.put(s, e, k::HEREDOC_TOKEN);
        if end > e {
            self.put(e, end, k::STR_END);
        }
        let label = Label::of(&b[s..e]).septets();
        let mut c = HEREDOC;
        if tabs {
            c |= HD_TABS;
        }
        if quote.is_some() {
            c |= HD_RAW;
        }
        let g = self.hd_at;
        if g < self.depth
            && g + 4 < self.depth
            && self.stack[g + 4] & !3 == HEREDOC
            && self.depth + 5 <= MAX
        {
            // a later opener on the line: its body after the earlier ones', its levels under theirs
            self.fill(end);
            self.stack.copy_within(g..self.depth, g + 5);
            for (i, s) in label.into_iter().enumerate() {
                self.stack[g + i] = DATA | s;
            }
            self.stack[g + 4] = c;
            self.depth += 5;
            self.derive_all();
        } else {
            for s in label {
                self.push(end, DATA | s);
            }
            self.push(end, c);
            self.hd_at = self.depth - 5;
            self.push(end, LINEMAIN);
            self.push(end, ARGS);
        }
        // a comment right after the word: the blanks before it the body's (syntect matches the
        // prototype in the preamble first)
        let mut q = end;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        if q > end && q < n && b[q] == b'#' {
            return Some(self.put(end, q, k::HEREDOC));
        }
        Some(end)
    }

    /// After `coproc` (at `e`): a coprocess's name and its `{ … }` (bat's `coproc-body`: the
    /// next word holds a `{` or a `{` follows it), its commands in braces of their own and
    /// arguments after them; else the command that follows is the coprocess.
    fn coproc(&mut self, e: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let blanks = |mut i: usize| {
            while i < n && is_space(b[i]) {
                i += 1;
            }
            i
        };
        let q = blanks(e);
        if q >= n {
            return e;
        }
        let mut w = q;
        while w < n && !is_space(b[w]) {
            w += 1;
        }
        let after = blanks(w);
        if !(b[q + 1..w].contains(&b'{') || after < n && b[after] == b'{') {
            return e;
        }
        // the name: up to the first place whose blanks end at `{`
        let mut r = q;
        while !(blanks(r) < n && b[blanks(r)] == b'{') {
            r += 1;
        }
        self.put(q, r, k::COPROC_NAME);
        let s = blanks(r);
        self.push(r, ARGS);
        self.put(s, s + 1, k::SECTION_BRACES_BEGIN);
        self.push(s + 1, FUNCBODY);
        s + 1
    }

    /// The here-document's word, when the line is its end: its levels popped, the line done.
    fn heredoc_end(&mut self) -> usize {
        let b = self.b;
        let n = b.len();
        let d = self.depth - 1;
        if d < 5 || self.stack[d - 4..d].iter().any(|&c| c & DATA == 0) {
            return self.heredoc_lost();
        }
        let label = Label::from_septets([0, 1, 2, 3].map(|i| self.stack[d - 4 + i]));
        let lead = if self.cur & HD_TABS != 0 {
            Lead::Blanks
        } else {
            Lead::None
        };
        let Some(c) = label.close(b, lead, Tail::Blanks) else {
            return 0;
        };
        if c.end == n {
            self.fill(c.start);
            self.put(c.start, n, k::HEREDOC_TOKEN);
        } else {
            self.fill(c.end);
            self.put(c.end, n, k::HEREDOC_SPACES);
        }
        for _ in 0..5 {
            self.pop(n);
        }
        n
    }

    /// A body whose word's levels the held levels cut (a line keeps [`crate::stack::HELD`]):
    /// nothing can end it, so it ends here, the line read in what was under it.
    #[cold]
    #[inline(never)]
    fn heredoc_lost(&mut self) -> usize {
        self.pop(0);
        while self.depth > 1 && self.cur & DATA != 0 {
            self.pop(0);
        }
        0
    }

    /// A here-document's body: expansions where its word wasn't quoted.
    fn heredoc(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if self.cur & HD_RAW != 0 {
            return n;
        }
        let e = find_in(b, p, n, *b"$`\\");
        if e >= n {
            return n;
        }
        match b[e] {
            b'\\' => {
                if matches!(b.get(e + 1), Some(b'`' | b'$' | b'"' | b'\\')) {
                    self.put(e, e + 2, k::BACKTICK_ESCAPE)
                } else {
                    e + 1
                }
            }
            _ => self.expand(e, X_EXP).unwrap_or(e + 1),
        }
    }

    // --- holders: a command's start ---

    fn holder(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if is_space(c) {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            return e;
        }
        if let Some(e) = self.proto(p) {
            return e;
        }
        if self.cur == CMDONLY && matches!(c, b')' | b'}' | b';' | b'|' | b'&' | b'`') {
            self.pop(p);
            return p;
        }
        // the holder's own closers
        match (self.cur, c) {
            (EXPCMD | PROC, b')') => {
                self.put(p, p + 1, k::PARENS_END);
                self.pop(p + 1);
                return p + 1;
            }
            (SUBSHELL, b')') => {
                self.put(p, p + 1, k::COMPOUND_END);
                self.set(p + 1, CMDONLY);
                self.push(p + 1, ARGS);
                return p + 1;
            }
            (TICK, b'`') => {
                self.put(p, p + 1, k::GROUP_END);
                self.pop(p + 1);
                return p + 1;
            }
            (BRACES, b'}') => {
                self.put(p, p + 1, k::BRACES_END);
                self.set(p + 1, CMDONLY);
                self.push(p + 1, ARGS);
                return p + 1;
            }
            (FUNCBODY, b'}') => {
                self.put(p, p + 1, k::SECTION_BRACES_END);
                self.pop(p + 1);
                return p + 1;
            }
            (FUNCBODY, b')') => {
                self.put(p, p + 1, k::COMPOUND_END);
                self.pop(p + 1);
                return p + 1;
            }
            (CASECMDS, b';') if matches!(b.get(p + 1), Some(b';' | b'&')) => {
                let e = if b[p + 1] == b';' && b.get(p + 2) == Some(&b'&') {
                    p + 3
                } else {
                    p + 2
                };
                self.put(p, e, k::CASE_END);
                self.pop(e);
                return e;
            }
            (CASECMDS, b'e') if has(b, p, b"esac") && breaks(b, p + 4) && self.word_starts(p) => {
                self.pop(p);
                return p;
            }
            _ => {}
        }
        match c {
            b';' | b'|' => {
                let e = self.post(p);
                return if e == p { p + 1 } else { e };
            }
            b'&' if b.get(p + 1) != Some(&b'>') => {
                let e = self.post(p);
                return if e == p { p + 1 } else { e };
            }
            b'`' if self.fl & BT != 0 => return p + 1,
            _ => {}
        }
        self.command(p)
    }

    /// Whether a word may start at `p` (`\b`): the byte before isn't a word's.
    #[inline(always)]
    fn word_starts(&self, p: usize) -> bool {
        p == 0 || !WORD[usize::from(self.b[p - 1])]
    }

    /// A command at `p`, not a blank: a function's definition, an assignment, a redirection,
    /// `!`, a keyword, a builtin, a compound command or a name.
    fn command(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        // the first word, as a keyword or builtin reads it
        let mut e = p;
        while e < n && WORD[usize::from(b[e])] {
            e += 1;
        }
        let (what, kind, o) = if e > p {
            lookup(b, p, e)
        } else if (c == b':' || c == b'.') && builtin_ends(b, p + 1) {
            e = p + 1;
            word(&b[p..e])
        } else {
            (W_NONE, 0, 0)
        };
        let only = self.cur == CMDONLY;
        if only {
            self.pop(p);
            return self.cmd(p, e, what, kind, o);
        }
        if what == W_FUNCTION && e < n && is_space(b[e]) && self.word_starts(p) {
            self.put(p, e, kind);
            self.push(e, FUNCPARENS);
            let mut q = e;
            while q < n && is_space(b[q]) {
                q += 1;
            }
            self.push(q, FUNCNAME);
            return q;
        }
        // the common start, a word of word bytes ending at a blank or the line's end: no
        // assignment (`=` would end it), a definition only where `(` follows the blanks
        let simple = e > p && (e == n || is_space(b[e]));
        // `name=`, `name+=`: an assignment (bat's `is_variable` reads the word to `=`), and no
        // definition (its name stops at `=`)
        // definition (its name stops at `=`); `+` breaks a keyword (`local+=`), `=` doesn't
        let assign = e > p
            && e < n
            && (b[e] == b'='
                || b[e] == b'+'
                    && b.get(e + 1) == Some(&b'=')
                    && !matches!(what, W_DECLARE | W_EXPORT | W_READONLY | W_ALIAS | W_COPROC));
        if assign {
            // the name's bytes are its (no string, escape or `[` in them): its context from
            // the operator on
            self.put(p, e, k::VAR_ASSIGN);
            self.push(e, VARNAME);
            return e;
        }
        if simple {
            if !c.is_ascii_digit() && self.parens_after(e) {
                self.put(p, e, k::FUNCTION_NAME);
                self.push(e, FUNCPARENS);
                return e;
            }
        } else if let Some(r) = self.defines(p) {
            return r;
        }
        if what == W_COPROC && breaks(b, e) {
            self.put(p, e, kind);
            return self.coproc(e);
        }
        if matches!(what, W_DECLARE | W_EXPORT | W_READONLY | W_ALIAS) && breaks(b, e) {
            self.put(p, e, kind);
            self.push(e, MORE);
            self.push(
                e,
                match what {
                    W_DECLARE => DECL,
                    W_EXPORT => DECL_EXPORT,
                    W_READONLY => DECL_READONLY,
                    _ => DECL_ALIAS,
                },
            );
            return e;
        }
        // an assignment whose name would be empty pops at once: at a backtick syntect's loop
        // guard passes over the empty pops to the value's unquoted string, the group in it; at
        // a metacharacter the assignment is passed over
        if c == b'`' && self.fl & BT == 0 && self.assigns(p) {
            self.push(p, VALUE);
            self.put(p, p + 1, k::GROUP_BEGIN);
            self.push(p + 1, TICK);
            return p + 1;
        }
        if !simple && !META[usize::from(c)] && c != b'`' && self.assigns(p) {
            // a command's assignments, then the command (`x=1 cmd`); `declare`'s go on
            // to more names ([`MORE`])
            self.push(p, VARNAME);
            return p;
        }
        // a redirection: digits, then `<`, `>`, `&>`
        let mut d = p;
        while d < n && b[d].is_ascii_digit() {
            d += 1;
        }
        if d < n
            && matches!(b[d], b'<' | b'>' | b'&')
            && let Some(r) = self.redirect(p, d)
        {
            return r;
        }
        if c == b'!' {
            return self.bang(p);
        }
        self.cmd(p, e, what, kind, o)
    }

    /// bat's `cmd` at `p`, its first word `p..e` read as `what`, `kind`, option set `o`.
    fn cmd(&mut self, p: usize, e: usize, what: u8, kind: u16, o: u8) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        // cmd: keywords, `((`, a closer's stop, `(`, builtins, `let`, `[[`, `[`, `{`, a name
        if e > p && breaks(b, e) && self.word_starts(p) {
            match what {
                W_IF => return self.put(p, e, kind),
                W_STAY => {
                    self.put(p, e, kind);
                    self.push(e, CMDONLY);
                    return e;
                }
                W_ARGS => {
                    self.put(p, e, kind);
                    self.push(e, ARGS);
                    return e;
                }
                W_FOR => {
                    self.put(p, e, kind);
                    self.push(e, FOR);
                    return e;
                }
                W_CASE => {
                    self.put(p, e, kind);
                    self.push(e, CASEBODY);
                    self.push(e, CASEWORD);
                    return e;
                }
                W_ESAC => return self.put(p, e, kind),
                _ => {}
            }
        }
        match c {
            b'(' if b.get(p + 1) == Some(&b'(') && self.close_from(p + 3) => {
                self.put(p, p + 2, k::ARITHMETIC_BEGIN);
                self.push(p + 2, ARITH);
                return p + 2;
            }
            b')' | b'}' => {
                // a closer that isn't this holder's: the character passed
                return p + 1;
            }
            b'(' => {
                self.put(p, p + 1, k::COMPOUND_BEGIN);
                self.push(p + 1, SUBSHELL);
                return p + 1;
            }
            _ => {}
        }
        if what == W_BUILTIN && builtin_ends(b, e) {
            self.put(p, e, kind);
            self.push(e, BARGS + o);
            return e;
        }
        if what == W_LET && breaks_word(b, e) && self.word_starts(p) {
            self.put(p, e, kind);
            self.push(e, LET);
            return e;
        }
        let spaced = |i: usize| i >= n || is_space(b[i]);
        match c {
            b'[' if b.get(p + 1) == Some(&b'[') && spaced(p + 2) => {
                self.put(p, p + 2, k::DOUBLE_BRACE_BEGIN);
                self.push(p + 2, POST);
                self.push(p + 2, DTEST);
                return p + 2;
            }
            b'[' if spaced(p + 1) => {
                self.put(p, p + 1, k::TEST_BEGIN);
                self.push(p + 1, POST);
                self.push(p + 1, TEST);
                return p + 1;
            }
            b'{' if spaced(p + 1) => {
                self.put(p, p + 1, k::BRACES_BEGIN);
                self.push(p + 1, BRACES);
                return p + 1;
            }
            b'`' if self.fl & BT == 0 => {
                // a name that ends before it starts: arguments, and the backtick in them
                self.push(p, ARGS);
                return p;
            }
            _ => {}
        }
        // a name, its arguments after it: a plain word at once, else its context (strings,
        // expansions, escapes in it; a comment after it, whose blanks are the name's)
        // past the word's bytes already read, which no name stops at
        let mut q = e.max(p);
        while q < n && !NAME_STOP[usize::from(b[q])] {
            q += 1;
        }
        if q > p
            && (q == n
                || !matches!(b[q], b'"' | b'\'' | b'$' | b'~' | b'%' | b'\\')
                    && !(is_space(b[q]) && self.comment_follows(q)))
        {
            self.put(p, q, COMMAND);
            self.push(q, ARGS);
            return q;
        }
        self.push(p, NAME);
        p
    }

    /// A function's definition at `p`, `name()` (bat's `is_function`): its name, then its
    /// parentheses and body.
    fn defines(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        if b[p].is_ascii_digit() || META[usize::from(b[p])] {
            return None;
        }
        let mut e = p + 1;
        while e < n && !META[usize::from(b[e])] && b[e] != b'=' {
            e += 1;
        }
        let mut q = e;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        if b.get(q) != Some(&b'(') {
            return None;
        }
        let mut r = q + 1;
        while r < n && is_space(b[r]) {
            r += 1;
        }
        if b.get(r) != Some(&b')') {
            return None;
        }
        self.put(p, e, k::FUNCTION_NAME);
        self.push(e, FUNCPARENS);
        Some(e)
    }

    /// Whether [`Scan::defines`] would read a definition at `p`.
    #[inline(always)]
    fn defines_at(&self, p: usize) -> bool {
        let b = self.b;
        let n = b.len();
        if b[p].is_ascii_digit() || META[usize::from(b[p])] {
            return false;
        }
        let mut e = p + 1;
        while e < n && !META[usize::from(b[e])] && b[e] != b'=' {
            e += 1;
        }
        let mut q = e;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        if b.get(q) != Some(&b'(') {
            return false;
        }
        let mut r = q + 1;
        while r < n && is_space(b[r]) {
            r += 1;
        }
        b.get(r) == Some(&b')')
    }

    /// Whether `\s*\(\s*\)` follows `e`.
    #[inline(always)]
    fn parens_after(&self, e: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let mut q = e;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        if q >= n || b[q] != b'(' {
            return false;
        }
        q += 1;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        q < n && b[q] == b')'
    }

    /// Whether an assignment starts at `p` (bat's `is_variable`): `[^{}()=\s]*`, a bracketed
    /// group of the same at most, then `=`.
    #[inline(always)]
    fn assigns(&self, p: usize) -> bool {
        let b = self.b;
        let n = b.len();
        let nbc = |mut i: usize| {
            while i < n && !matches!(b[i], b'{' | b'}' | b'(' | b')' | b'=') && !is_space(b[i]) {
                i += 1;
            }
            i
        };
        let mut i = nbc(p);
        if i < n && matches!(b[i], b'(' | b'{') {
            i = nbc(i + 1);
            if i >= n || !matches!(b[i], b')' | b'}') {
                return false;
            }
            i = nbc(i + 1);
        }
        i < n && b[i] == b'='
    }

    /// `!` at a command's start: the logical not, a history expansion, or its mark.
    fn bang(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if p + 1 >= n || is_space(b[p + 1]) {
            return self.put(p, p + 1, k::LOGICAL);
        }
        let mut e = p + 1;
        if b[e] == b'!' {
            e += 1;
        } else {
            if b[e] == b'-' {
                e += 1;
            }
            let s = e;
            while e < n && b[e].is_ascii_digit() {
                e += 1;
            }
            if e == s {
                return self.put(p, p + 1, k::HISTORY_MARK);
            }
        }
        self.put(p, p + 1, k::HISTORY_MARK);
        self.put(p + 1, e, k::HISTORY)
    }

    // --- a command's name ---

    fn name(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if is_space(c)
            && let Some(e) = self.comment_after(p)
        {
            return e;
        }
        if NAME_STOP[usize::from(c)] {
            match c {
                b'"' | b'\'' | b'$' | b'~' | b'%' => {
                    if let Some(e) = self.expand(p, X_STR | X_TILDE | X_JOB) {
                        return e;
                    }
                    return p + 1;
                }
                b'\\' => return self.backslash(p),
                _ => {
                    self.set(p, ARGS);
                    return p;
                }
            }
        }
        let mut e = p + 1;
        while e < n && !NAME_STOP[usize::from(b[e])] {
            e += 1;
        }
        e
    }

    // --- arguments ---

    /// A command's arguments' tokens while the context stays, without the loop's dispatch
    /// between them.
    fn args(&mut self, mut p: usize) -> usize {
        let c = self.cur;
        let n = self.b.len();
        while p < n && self.cur == c {
            let q = self.args_one(p);
            if q == p {
                break;
            }
            p = q;
        }
        p
    }

    /// A command's arguments ([`ARGS`], a builtin's [`BARGS`]), and the contexts that share
    /// their boilerplate: [`EOO`], [`MORE`], [`FOR`], [`TEST`], [`POST`]: a token.
    #[inline(always)]
    fn args_one(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let cur = self.cur;
        if cur == POST {
            return self.post(p);
        }
        // a blank run, and what may follow one: a comment, `--`, an option, `]`, `]]`
        if is_space(c) || p == 0 {
            let mut e = p;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if e == n {
                return e;
            }
            let lead = e > p || p == 0;
            if lead {
                match b[e] {
                    b'#' => {
                        if cur == MORE {
                            self.pop(e);
                            return e;
                        }
                        return self.comment(e);
                    }
                    b'-' => {
                        if let Some(r) = self.dash(p, e) {
                            return r;
                        }
                    }
                    b']' if cur == TEST && e > p => {
                        self.put(e, e + 1, k::TEST_END);
                        self.pop(e + 1);
                        return e + 1;
                    }
                    b'+' if cur == BARGS + O_SET => {
                        if let Some(r) = self.dash(p, e) {
                            return r;
                        }
                    }
                    _ if (BARGS..DATA).contains(&cur)
                        && OPTS[usize::from(cur - BARGS)].bare != 0 =>
                    {
                        if let Some(r) = self.bare(e) {
                            return r;
                        }
                    }
                    _ => {}
                }
            }
            if e > p {
                return e;
            }
        }
        // the boilerplate: `)`, `}`, a redirection, the operators
        match c {
            b')' | b'}' => {
                self.pop(p);
                return p;
            }
            b'`' if self.fl & BT != 0 || cur == MORE => {
                self.pop(p);
                return p;
            }
            b';' | b'|' | b'&' | b'<' | b'>' => {
                if matches!(c, b'<' | b'>' | b'&') {
                    let d = self.digits_before(p);
                    if let Some(r) = self.redirect(d, p) {
                        return r;
                    }
                }
                return self.operator(p);
            }
            _ => {}
        }
        if let Some(e) = self.proto(p) {
            return e;
        }
        if let Some(e) = self.expand(p, X_ALL) {
            return e;
        }
        match cur {
            MORE => {
                // `(?=\S)`: another assignment's name (at `(` an empty one, its value an
                // array's)
                self.push(p, VARNAME);
                return p;
            }
            FOR => {
                if c == b'i' && has(b, p, b"in") && breaks(b, p + 2) && self.word_starts(p) {
                    return self.put(p, p + 2, k::IN);
                }
                if c == b'(' && b.get(p + 1) == Some(&b'(') && self.close_from(p + 3) {
                    self.put(p, p + 2, k::ARITHMETIC_BEGIN);
                    self.push(p + 2, ARITH);
                    return p + 2;
                }
            }
            TEST => {
                if let Some(e) = self.test_op(p) {
                    return e;
                }
            }
            _ => {}
        }
        // plain text
        let mut e = p + 1;
        while e < n && !ARG_STOP[usize::from(b[e])] {
            e += 1;
        }
        // digits before a redirection are its descriptor
        e
    }

    /// Where the digits before `p` start, past the last token.
    #[inline(always)]
    fn digits_before(&self, p: usize) -> usize {
        let b = self.b;
        let mut d = p;
        while d > self.at && b[d - 1].is_ascii_digit() {
            d -= 1;
        }
        d
    }

    /// `;`, `|`, `&` (and `<`, `>` that redirect nothing) at `p`: the command ends, and the
    /// operator is cmd-post's.
    fn operator(&mut self, p: usize) -> usize {
        let b = self.b;
        let c = b[p];
        if !matches!(c, b';' | b'|' | b'&') {
            return p + 1;
        }
        // pop the command's contexts down to its holder
        self.pop(p);
        while self.cur > LAST_HOLDER && ends_line(self.cur) && self.depth > 1 {
            self.pop(p);
        }
        self.post(p)
    }

    /// bat's `cmd-post` at `p`: an operator ends the command, else the holder goes on.
    fn post(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if is_space(c) {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if e < n && b[e] == b'#' {
                return self.comment(e);
            }
            return e;
        }
        if c == b'\\' {
            return self.backslash(p);
        }
        if self.cur == POST {
            self.pop(p);
        }
        let next = b.get(p + 1).copied();
        let (e, kind) = match c {
            b';' if !matches!(next, Some(b';' | b'&')) => (p + 1, k::CONTINUE),
            b'|' if next == Some(b'|') => (p + 2, k::OR),
            b'|' => (p + 1, k::PIPE),
            b'&' if next == Some(b'&') => (p + 2, k::AND),
            b'&' if next != Some(b'>') => (p + 1, k::JOB),
            _ => return p,
        };
        self.put(p, e, kind)
    }

    /// `-` (or `set`'s `+`) at `e` after a blank run from `p`: `--` ending the options, an
    /// option, a builtin's option, a test's operator.
    fn dash(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let cur = self.cur;
        if cur == TEST {
            return self.test_op(e);
        }
        if cur != EOO
            && b[e] == b'-'
            && b.get(e + 1) == Some(&b'-')
            && (e + 2 >= n || is_space(b[e + 2]))
        {
            self.put(p, e + 2, k::END_OF_OPTIONS);
            self.set(e + 2, EOO);
            return Some(e + 2);
        }
        if cur == ARGS {
            // `(?:\s+|^)--?(?=[\w$])`
            let mut q = e + 1;
            if b.get(q) == Some(&b'-') {
                q += 1;
            }
            if q < n && (WORD[usize::from(b[q])] || b[q] == b'$') {
                self.put(p, q, k::OPTION_MARK);
                self.push(q, OPTION);
                return Some(q);
            }
            return None;
        }
        if (BARGS..DATA).contains(&cur) {
            let o = cur - BARGS;
            let spec = &OPTS[usize::from(o)];
            if spec.dash == 0 {
                return None;
            }
            let mut q = e + 1;
            if o == O_SET && q < n && b[q] == b'o' && option_ends(b, q + 1) {
                q += 1;
            } else if spec.many {
                while q < n && in_set(spec.dash, b[q]) {
                    q += 1;
                }
                if q == e + 1 || !option_ends(b, q) {
                    return None;
                }
            } else {
                if q >= n || !in_set(spec.dash, b[q]) || !option_ends(b, q + 1) {
                    return None;
                }
                q += 1;
            }
            if b[e] == b'+' && o != O_SET {
                return None;
            }
            self.put(e, e + 1, k::OPTION_MARK);
            return Some(self.put(e + 1, q, k::OPTION));
        }
        None
    }

    /// A builtin's bare option (`[chars]+` with no `-`) at `e`, after a blank run.
    fn bare(&mut self, e: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let spec = &OPTS[usize::from(self.cur - BARGS)];
        let mut q = e;
        while q < n && in_set(spec.bare, b[q]) {
            q += 1;
        }
        if q == e || !option_ends(b, q) {
            return None;
        }
        Some(self.put(e, q, k::OPTION))
    }

    /// An option's name: word characters and `-`, `$` expansions, up to `=` (its assignment)
    /// or any other byte.
    fn option(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if is_space(c)
            && let Some(e) = self.comment_after(p)
        {
            return e;
        }
        if WORD[usize::from(c)] || c == b'-' {
            let mut e = p + 1;
            while e < n && (WORD[usize::from(b[e])] || b[e] == b'-') {
                e += 1;
            }
            return e;
        }
        match c {
            b'=' => {
                self.put(p, p + 1, k::OPTION_ASSIGN);
                self.pop(p + 1);
                p + 1
            }
            b'$' => match self.dollar(p, X_STR) {
                Some(e) => e,
                None => {
                    // a `$` alone: the option's text
                    p + 1
                }
            },
            _ => {
                self.pop(p);
                p
            }
        }
    }

    /// `[ … ]`'s and `[[ … ]]`'s operators at `p` (bat's `expression-test`).
    fn test_op(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let spaced = |i: usize| i >= n || is_space(b[i]);
        match c {
            b'-' => {
                let q = p + 1;
                if q < n && b"aobcdefghknoprstuvwxzGLNORS".contains(&b[q]) && spaced(q + 1) {
                    self.put(p, p + 1, k::OPTION_MARK);
                    return Some(self.put(q, q + 1, k::OPTION));
                }
                if q + 1 < n
                    && [
                        b"ef", b"nt", b"ot", b"eq", b"ne", b"lt", b"le", b"gt", b"ge",
                    ]
                    .iter()
                    .any(|w| b[q..q + 2] == **w)
                    && spaced(q + 2)
                {
                    self.put(p, p + 1, k::OPTION_MARK);
                    return Some(self.put(q, q + 2, k::OPTION));
                }
                None
            }
            b'=' if b.get(p + 1) == Some(&b'~') => {
                let mut e = p + 2;
                self.put(p, e, k::LOGICAL);
                while e < n && is_space(b[e]) {
                    e += 1;
                }
                self.push(e, REGEX);
                Some(e)
            }
            b'=' | b'!' => {
                let e = if b.get(p + 1) == Some(&b'=') {
                    p + 2
                } else {
                    p + 1
                };
                Some(self.put(p, e, k::LOGICAL))
            }
            b'<' | b'>' => Some(self.put(p, p + 1, k::LOGICAL)),
            b'|' if b.get(p + 1) == Some(&b'|') => Some(self.put(p, p + 2, k::LOGICAL)),
            b'&' if b.get(p + 1) == Some(&b'&') => Some(self.put(p, p + 2, k::LOGICAL)),
            _ => None,
        }
    }

    /// `[[ … ]]` ([`DTEST`]) and `=~`'s pattern in it ([`REGEX`]).
    fn dtest(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if self.cur == REGEX {
            if is_space(c) {
                self.pop(p);
                return p;
            }
            if let Some(e) = self.expand(p, X_ALL) {
                return e;
            }
            let mut e = p + 1;
            while e < n && !ARG_STOP[usize::from(b[e])] && !is_space(b[e]) {
                e += 1;
            }
            return e;
        }
        if is_space(c) {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if has(b, e, b"]]") {
                self.put(e, e + 2, k::DOUBLE_BRACE_END);
                self.pop(e + 2);
                return e + 2;
            }
            if e < n && b[e] == b'#' {
                return self.comment(e);
            }
            return e;
        }
        if p == 0 && c == b'#' {
            return self.comment(p);
        }
        if c == b'\\' {
            return self.backslash(p);
        }
        if let Some(e) = self.expand(p, X_ALL) {
            return e;
        }
        if let Some(e) = self.test_op(p) {
            return e;
        }
        let mut e = p + 1;
        while e < n && !TEST_STOP[usize::from(b[e])] {
            e += 1;
        }
        e
    }

    // --- assignments ---

    /// `declare`'s (and the others') options, then the first name.
    fn decl(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut q = p;
        while q < n && is_space(b[q]) {
            q += 1;
        }
        if q < n && b[q] == b'-' {
            let set: &[u8] = match self.cur {
                DECL => b"aAfFgilnrtux",
                DECL_EXPORT => b"fn",
                DECL_READONLY => b"aAf",
                _ => b"",
            };
            let mut e = q + 1;
            while e < n && set.contains(&b[e]) {
                e += 1;
            }
            if e == q + 1 && b.get(e) == Some(&b'p') {
                e += 1;
            }
            if e > q + 1 {
                self.put(q, q + 1, k::OPTION_MARK);
                return self.put(q + 1, e, k::OPTION);
            }
        }
        let name = if self.cur == DECL_ALIAS {
            ALIASNAME
        } else {
            VARNAME
        };
        self.set(q, name);
        q
    }

    /// An assignment's name, up to `=`, `+=`, `-=`, `?=`, a blank or a metacharacter; then
    /// its operator and value.
    fn varname(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let alias = self.cur == ALIASNAME;
        let ends = |i: usize| {
            let c = b[i];
            c == b'='
                || matches!(c, b'+' | b'-' | b'?') && b.get(i + 1) == Some(&b'=')
                || is_space(c)
                || !alias && (META[usize::from(c)] || c == b'`')
        };
        if ends(p) {
            let e = match c {
                b'=' => p + 1,
                b'+' | b'-' | b'?' if !is_space(c) => p + 2,
                _ => p,
            };
            if e > p {
                self.put(p, e, k::ASSIGN);
            }
            // the value
            // the value: bat's `vardef-value` tries `(` first, with or without the operator
            match b.get(e) {
                Some(b'(') => {
                    self.put(e, e + 1, k::PARENS_BEGIN);
                    self.set(e + 1, ARRAYVAL);
                    return e + 1;
                }
                Some(b'&' | b'`') | None => {
                    self.pop(e);
                    return e;
                }
                _ if e > p => {
                    self.set(e, VALUE);
                    return e;
                }
                _ => {
                    self.pop(e);
                    return e;
                }
            }
        }
        match c {
            b'[' => {
                self.put(p, p + 1, k::SECTION_BRACES_BEGIN);
                self.push(p + 1, INDEX);
                p + 1
            }
            b'"' | b'\'' => self.expand(p, X_STR).unwrap_or(p + 1),
            b'$' if matches!(b.get(p + 1), Some(b'\'' | b'"')) => {
                self.expand(p, X_STR).unwrap_or(p + 1)
            }
            b'\\' => self.backslash(p),
            _ => {
                let mut e = p + 1;
                while e < n
                    && !matches!(
                        b[e],
                        b'[' | b'"' | b'\'' | b'\\' | b'=' | b'+' | b'-' | b'?' | b'$'
                    )
                    && !is_space(b[e])
                    && (alias || !META[usize::from(b[e])] && b[e] != b'`')
                {
                    e += 1;
                }
                e
            }
        }
    }

    /// An assignment's unquoted value, up to a metacharacter or a backtick.
    fn value(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if META[usize::from(c)] || c == b'`' {
            self.pop(p);
            return p;
        }
        if c == b'\\' {
            return self.backslash(p);
        }
        if let Some(e) = self.expand(p, X_ALL) {
            return e;
        }
        let mut e = p + 1;
        while e < n && !VALUE_STOP[usize::from(b[e])] {
            e += 1;
        }
        e
    }

    /// `x=(…)` ([`ARRAYVAL`]) and `[…]=` in it ([`ARRAYIDX`]).
    fn array(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if self.cur == ARRAYIDX && c == b']' {
            self.put(p, p + 1, k::BRACKETS_END);
            if b.get(p + 1) == Some(&b'=') {
                self.put(p + 1, p + 2, k::ASSIGN);
                self.pop(p + 2);
                return p + 2;
            }
            self.pop(p + 1);
            return p + 1;
        }
        if self.cur == ARRAYVAL {
            match c {
                b')' => {
                    self.put(p, p + 1, k::PARENS_END);
                    self.pop(p + 1);
                    return p + 1;
                }
                b'[' => {
                    self.put(p, p + 1, k::BRACKETS_BEGIN);
                    self.push(p + 1, ARRAYIDX);
                    return p + 1;
                }
                _ => {}
            }
            if is_space(c) {
                let mut e = p + 1;
                while e < n && is_space(b[e]) {
                    e += 1;
                }
                if e < n && b[e] == b'#' {
                    return self.comment(e);
                }
                return e;
            }
            if let Some(e) = self.proto(p) {
                return e;
            }
        }
        if let Some(e) = self.expand(p, X_ALL) {
            return e;
        }
        let mut e = p + 1;
        while e < n && !ARG_STOP[usize::from(b[e])] && b[e] != b']' {
            e += 1;
        }
        e
    }

    // --- functions ---

    /// `function`'s name: up to `(`, `{` (blanks before them its parentheses') or the line's
    /// end.
    fn funcname(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let mut e = p;
        while e < n && !matches!(b[e], b'(' | b'{') {
            if is_space(b[e]) {
                let mut q = e;
                while q < n && is_space(b[q]) {
                    q += 1;
                }
                if q < n && matches!(b[q], b'(' | b'{' | b'#') {
                    break;
                }
                e = q;
                continue;
            }
            e += 1;
        }
        self.pop(e);
        e
    }

    /// Between a function's name and its body: `()`, then `{` or `(`.
    fn funcparens(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        match c {
            b'(' => {
                let mut q = p + 1;
                while q < n && is_space(b[q]) {
                    q += 1;
                }
                if b.get(q) == Some(&b')') {
                    self.put(p, p + 1, k::PARENS_BEGIN);
                    return self.put(q, q + 1, k::PARENS_END);
                }
                self.put(p, p + 1, k::COMPOUND_BEGIN);
                self.set(p + 1, FUNCBODY);
                p + 1
            }
            b'{' => {
                self.put(p, p + 1, k::SECTION_BRACES_BEGIN);
                self.set(p + 1, FUNCBODY);
                p + 1
            }
            _ => {
                if let Some(e) = self.proto(p) {
                    return e;
                }
                p + 1
            }
        }
    }

    // --- case ---

    fn case(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let cur = self.cur;
        if is_space(c) {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if e < n && b[e] == b'#' {
                return self.comment(e);
            }
            return e;
        }
        if let Some(e) = self.proto(p) {
            return e;
        }
        let esac = c == b'e' && has(b, p, b"esac") && breaks(b, p + 4) && self.word_starts(p);
        match cur {
            CASEWORD => {
                if c == b'i' && has(b, p, b"in") && breaks(b, p + 2) && self.word_starts(p) {
                    self.put(p, p + 2, k::IN);
                    self.pop(p + 2);
                    return p + 2;
                }
                if esac {
                    self.pop(p);
                    return p;
                }
                if let Some(e) = self.expand(p, X_ALL) {
                    return e;
                }
            }
            CASEBODY => {
                if esac {
                    self.put(p, p + 4, k::COND_END);
                    self.pop(p + 4);
                    return p + 4;
                }
                if c == b'(' {
                    self.put(p, p + 1, k::PATTERNS_BEGIN);
                    self.push(p + 1, CASEPAT);
                    return p + 1;
                }
                self.push(p, CASEPAT);
                return p;
            }
            _ => {
                if cur == CASEPAT {
                    if c == b')' {
                        self.put(p, p + 1, k::PATTERNS_END);
                        self.set(p + 1, CASECMDS);
                        return p + 1;
                    }
                    if c == b';' && matches!(b.get(p + 1), Some(b';' | b'&')) {
                        self.set(p, CASECMDS);
                        return p;
                    }
                    if esac {
                        self.pop(p);
                        return p;
                    }
                } else if c == b')' {
                    self.put(p, p + 1, k::PARENS_END);
                    self.pop(p + 1);
                    return p + 1;
                }
                match c {
                    b'|' => return self.put(p, p + 1, k::LOGICAL),
                    b'(' => {
                        self.put(p, p + 1, k::PARENS_BEGIN);
                        self.push(p + 1, PATPARENS);
                        return p + 1;
                    }
                    _ => {}
                }
                if let Some(e) = self.expand(p, X_STR | X_PAT | X_TILDE | X_CMD_FIRST) {
                    return e;
                }
            }
        }
        let mut e = p + 1;
        while e < n && !ARG_STOP[usize::from(b[e])] && b[e] != b'e' && b[e] != b'i' {
            e += 1;
        }
        e
    }

    // --- expressions ---

    /// `(( … ))`, `$(( … ))`, `let`, `(…)` in them, `[…]` of an array, `${x:…}`.
    fn expr(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let cur = self.cur;
        match c {
            b')' if b.get(p + 1) == Some(&b')') && matches!(cur, ARITH | XARITH) => {
                let kind = if cur == ARITH {
                    k::ARITHMETIC_END
                } else {
                    k::PARENS_END
                };
                self.put(p, p + 2, kind);
                self.pop(p + 2);
                return p + 2;
            }
            b')' if cur == PARENS => {
                self.put(p, p + 1, k::PARENS_END);
                self.pop(p + 1);
                return p + 1;
            }
            b']' if cur == INDEX => {
                self.put(p, p + 1, k::SECTION_BRACES_END);
                self.pop(p + 1);
                return p + 1;
            }
            b'*' | b'@' if cur == INDEX => return self.put(p, p + 1, k::ARRAY_VAR),
            b':' if cur == SUBSTR => {
                self.put(p, p + 1, k::SUBSTRING_END);
                self.set(p + 1, SUBSTR2);
                return p + 1;
            }
            b'}' if matches!(cur, SUBSTR | SUBSTR2) => {
                self.put(p, p + 1, k::PARAMETER_END);
                self.pop(p + 1);
                return p + 1;
            }
            b'`' if cur == LET && self.fl & BT != 0 => {
                self.pop(p);
                return p;
            }
            _ => {}
        }
        if is_space(c) {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if e < n && b[e] == b'#' && cur != INDEX {
                return self.comment(e);
            }
            return e;
        }
        if c == b'\\' {
            return self.backslash(p);
        }
        if c == b'#' && p == 0 {
            return self.comment(p);
        }
        let at = |i: usize| b.get(i).copied().unwrap_or(b'\n');
        // numbers, at a word's start
        if c.is_ascii_digit() && self.word_starts(p) {
            return self.number(p);
        }
        if WORD[usize::from(c)] {
            let mut e = p + 1;
            while e < n && WORD[usize::from(b[e])] {
                e += 1;
            }
            return e;
        }
        let (e, kind) = match c {
            // `[*/%+\-&^|]?=|<<=|>>=`
            b'*' | b'/' | b'%' | b'+' | b'-' | b'&' | b'^' | b'|' if at(p + 1) == b'=' => {
                (p + 2, k::ASSIGN)
            }
            b'=' => (p + 1, k::ASSIGN),
            b'<' if at(p + 1) == b'<' && at(p + 2) == b'=' => (p + 3, k::ASSIGN),
            b'>' if at(p + 1) == b'>' && at(p + 2) == b'=' => (p + 3, k::ASSIGN),
            // `\+\+?|\-\-?|\*\*?|%|/`
            b'+' | b'-' | b'*' => {
                let e = if at(p + 1) == c { p + 2 } else { p + 1 };
                (e, k::ARITH_OP)
            }
            b'%' | b'/' => (p + 1, k::ARITH_OP),
            // `<[=<]?|>[=>]?|[=!]=|&&|\:|\|\||!`
            b'<' => (
                if matches!(at(p + 1), b'=' | b'<') {
                    p + 2
                } else {
                    p + 1
                },
                k::LOGICAL,
            ),
            b'>' => (
                if matches!(at(p + 1), b'=' | b'>') {
                    p + 2
                } else {
                    p + 1
                },
                k::LOGICAL,
            ),
            b'!' if at(p + 1) == b'=' => (p + 2, k::LOGICAL),
            b'&' if at(p + 1) == b'&' => (p + 2, k::LOGICAL),
            b'|' if at(p + 1) == b'|' => (p + 2, k::LOGICAL),
            b':' | b'!' => (p + 1, k::LOGICAL),
            b'&' | b'|' | b'^' | b'~' => (p + 1, k::BITWISE),
            b',' | b';' => (p + 1, k::SEPARATOR),
            b'?' => (p + 1, k::TERNARY),
            b'(' => {
                self.put(p, p + 1, k::PARENS_BEGIN);
                self.push(p + 1, PARENS);
                return p + 1;
            }
            _ => {
                if matches!(cur, SUBSTR | SUBSTR2) {
                    if let Some(e) = self.expand(p, X_STR | X_TILDE) {
                        return e;
                    }
                } else if let Some(e) = self.expand(p, X_STR) {
                    return e;
                }
                let mut e = p + 1;
                while e < n && !EXPR_STOP[usize::from(b[e])] && !WORD[usize::from(b[e])] {
                    e += 1;
                }
                return e;
            }
        };
        self.put(p, e.min(n), kind)
    }

    /// A number at `p`: `0x…`, `0…` (octal), `base#n`, decimal.
    fn number(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'x' | b'X')) {
            self.put(p, p + 2, k::HEX_BASE);
            let mut e = p + 2;
            while e < n && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            self.put(p + 2, e, k::HEX);
            if e < n && b[e].is_ascii_alphabetic() {
                return self.put(e, e + 1, k::NOT_HEX);
            }
            return e;
        }
        if b[p] == b'0' && matches!(b.get(p + 1), Some(b'0'..=b'7')) {
            self.put(p, p + 1, k::OCTAL_BASE);
            let mut e = p + 1;
            while e < n && matches!(b[e], b'0'..=b'7') {
                e += 1;
            }
            self.put(p + 1, e, k::OCTAL);
            if e < n && matches!(b[e], b'8' | b'9') {
                return self.put(e, e + 1, k::NOT_OCTAL);
            }
            return e;
        }
        let mut e = p;
        while e < n && b[e].is_ascii_digit() {
            e += 1;
        }
        if e < n && b[e] == b'#' {
            let mut q = e + 1;
            while q < n && (b[q].is_ascii_alphanumeric() || b[q] == b'@' || b[q] == b'_') {
                q += 1;
            }
            if q > e + 1 {
                self.put(p, e + 1, k::OTHER_BASE_MARK);
                return self.put(e + 1, q, k::OTHER_BASE);
            }
        }
        self.put(p, e, k::DECIMAL)
    }

    // --- `${…}` ---

    fn param(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        let cur = self.cur;
        if c == b'}' {
            self.put(p, p + 1, k::PARAMETER_END);
            self.pop(p + 1);
            return p + 1;
        }
        let at = |i: usize| b.get(i).copied().unwrap_or(b'\n');
        if cur == PARAM {
            // the operators after the name
            let star = matches!(c, b'@' | b'*');
            let q = if star { p + 1 } else { p };
            let o = at(q);
            if o == b'/' {
                if star {
                    self.put(p, q, k::LANG_VAR);
                }
                self.put(q, q + 1, k::SUBSTITUTION);
                let mut r = q + 1;
                if matches!(at(r), b'/' | b'#' | b'%') {
                    r = self.put(r, r + 1, k::SWITCH);
                }
                self.set(r, PARAMPAT);
                return r;
            }
            if !star
                && (matches!(c, b'-' | b'+' | b'=' | b'?')
                    || c == b':' && matches!(at(p + 1), b'-' | b'+' | b'=' | b'?'))
            {
                let e = if c == b':' { p + 2 } else { p + 1 };
                self.put(p, e, k::ASSIGN);
                self.set(e, PARAMC);
                return e;
            }
            if o == b':' && (star && c == b'@' || !star) {
                if c == b'@' {
                    self.put(p, q, k::LANG_VAR);
                }
                self.put(q, q + 1, k::SUBSTRING_BEGIN);
                self.set(q + 1, SUBSTR);
                return q + 1;
            }
            if c == b'#' && at(p + 1) == b'}' {
                return p + 1;
            }
            if matches!(o, b'#' | b'%' | b'^' | b',') {
                let e = if at(q + 1) == o { q + 2 } else { q + 1 };
                if star {
                    self.put(p, q, k::LANG_VAR);
                }
                self.put(q, e, k::EXPANSION_OP);
                self.set(e, PARAMX);
                return e;
            }
            if o == b'@'
                && matches!(at(q + 1), b'Q' | b'E' | b'P' | b'A' | b'a')
                && at(q + 2) == b'}'
            {
                if star {
                    self.put(p, q, k::LANG_VAR);
                }
                self.put(q, q + 1, k::EXPANSION_OP);
                return self.put(q + 1, q + 2, k::SWITCH);
            }
            if c == b'[' {
                self.put(p, p + 1, k::SECTION_BRACES_BEGIN);
                self.push(p + 1, INDEX);
                return p + 1;
            }
            if star && at(p + 1) == b'}' {
                return self.put(p, p + 1, k::LANG_VAR);
            }
        }
        if cur == PARAMPAT && c == b'/' {
            self.put(p, p + 1, k::SUBSTITUTION);
            self.set(p + 1, PARAMC);
            return p + 1;
        }
        if c == b'\\' {
            return self.backslash(p);
        }
        if is_space(c) && cur != PARAMX {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            if e < n && b[e] == b'#' && cur != PARAMPAT {
                return self.comment(e);
            }
            return e;
        }
        let what = if matches!(cur, PARAMPAT | PARAMX) {
            X_STR | X_TILDE | X_PAT
        } else {
            X_STR | X_TILDE
        };
        if let Some(e) = self.expand(p, what) {
            return e;
        }
        let mut e = p + 1;
        while e < n
            && !matches!(
                b[e],
                b'}' | b'"'
                    | b'\''
                    | b'$'
                    | b'`'
                    | b'\\'
                    | b'~'
                    | b'/'
                    | b'#'
                    | b'%'
                    | b'^'
                    | b','
                    | b':'
                    | b'-'
                    | b'+'
                    | b'='
                    | b'?'
                    | b'@'
                    | b'*'
                    | b'['
                    | b' '
                    | b'\t'
            )
        {
            e += 1;
        }
        e
    }

    // --- patterns ---

    /// `?(…)` ([`EXTGLOB`]), a pattern's `[…]` ([`SET`]), `{a,b}` ([`BRACEX`]).
    fn pattern(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        match self.cur {
            EXTGLOB => match c {
                b')' => {
                    self.put(p, p + 1, k::PARENS_END);
                    self.pop(p + 1);
                    return p + 1;
                }
                b'|' => return self.put(p, p + 1, k::OR),
                _ => {}
            },
            SET => {
                if c == b']' || c == b'-' && b.get(p + 1) == Some(&b']') {
                    let e = if c == b'-' { p + 1 } else { p };
                    self.put(e, e + 1, k::SET_END);
                    self.pop(e + 1);
                    return e + 1;
                }
                if c == b'-' {
                    return self.put(p, p + 1, k::WORD_OP);
                }
                let at = |i: usize| b.get(i).copied().unwrap_or(b'\n');
                if c == b'.' && WORD[usize::from(at(p + 1))] && at(p + 2) == b'.' {
                    self.put(p, p + 1, k::COLLATE_BEGIN);
                    return self.put(p + 2, p + 3, k::COLLATE_END);
                }
                if c == b'=' && WORD[usize::from(at(p + 1))] && at(p + 2) == b'=' {
                    self.put(p, p + 1, k::EQUIVALENCE_BEGIN);
                    return self.put(p + 2, p + 3, k::EQUIVALENCE_END);
                }
                if c == b':' {
                    let mut e = p + 1;
                    while e < n && b[e].is_ascii_lowercase() {
                        e += 1;
                    }
                    if e > p + 1 && at(e) == b':' {
                        self.put(p, p + 1, k::CLASS_BEGIN);
                        return self.put(e, e + 1, k::CLASS_END);
                    }
                }
                if c == b'[' && !matches!(at(p + 1), b'.' | b'=' | b':') {
                    return p + 1;
                }
            }
            _ => match c {
                b'}' => {
                    self.put(p, p + 1, k::BRACE_END);
                    self.pop(p + 1);
                    return p + 1;
                }
                b',' => return self.put(p, p + 1, k::SEPARATOR),
                _ => {}
            },
        }
        if is_space(c) {
            let mut e = p + 1;
            while e < n && is_space(b[e]) {
                e += 1;
            }
            return e;
        }
        if let Some(e) = self.proto(p) {
            return e;
        }
        if let Some(e) = self.expand(p, X_ALL) {
            return e;
        }
        let mut e = p + 1;
        while e < n
            && !ARG_STOP[usize::from(b[e])]
            && !matches!(b[e], b',' | b'-' | b'.' | b':' | b'|' | b'=')
        {
            e += 1;
        }
        e
    }

    /// A redirection's target, up to a metacharacter or a backtick.
    fn target(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b[p];
        if META[usize::from(c)] || c == b'`' {
            self.pop(p);
            return p;
        }
        if c == b'\\' {
            return self.backslash(p);
        }
        if let Some(e) = self.expand(p, X_ALL) {
            return e;
        }
        let mut e = p + 1;
        while e < n && !ARG_STOP[usize::from(b[e])] {
            e += 1;
        }
        e
    }

    // --- strings ---

    /// A string's pieces while it stays on top.
    fn dq(&mut self, mut p: usize) -> usize {
        let c = self.cur;
        let n = self.b.len();
        while p < n && self.cur == c {
            let q = self.dq_one(p);
            if q == p {
                break;
            }
            p = q;
        }
        p
    }

    /// `"…"` and `$"…"`: escapes, parameters, substitutions.
    #[inline(always)]
    fn dq_one(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let e = find_in(b, p, n, *b"\"$`\\");
        if e >= n {
            return n;
        }
        match b[e] {
            b'"' => {
                self.put(e, e + 1, k::STR_END);
                self.pop(e + 1);
                e + 1
            }
            b'\\' => {
                if e + 1 == n {
                    return self.put(e, n, k::ESCAPE);
                }
                if matches!(b[e + 1], b'$' | b'`' | b'"' | b'\\') {
                    return self.put(e, e + 2, k::ESCAPE);
                }
                e + 1
            }
            _ => self.expand(e, X_EXP).unwrap_or(e + 1),
        }
    }

    fn sq(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let e = find_in(b, p, n, *b"'");
        if e >= n {
            return n;
        }
        self.put(e, e + 1, k::STR_END);
        self.pop(e + 1);
        e + 1
    }

    /// `$'…'`: its escapes.
    fn ansi(&mut self, p: usize) -> usize {
        let b = self.b;
        let n = b.len();
        let e = find_in(b, p, n, *b"'\\");
        if e >= n {
            return n;
        }
        if b[e] == b'\'' {
            self.put(e, e + 1, k::STR_END);
            self.pop(e + 1);
            return e + 1;
        }
        if e + 1 == n {
            return self.put(e, n, k::ESCAPE);
        }
        let c = b[e + 1];
        let len = match c {
            b'$' | b'`' | b'"' | b'\\' | b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\''
            | b'?' => 2,
            b'0'..=b'8' => {
                let mut q = e + 1;
                while q < n && q < e + 4 && matches!(b[q], b'0'..=b'8') {
                    q += 1;
                }
                q - e
            }
            b'x' => {
                let mut q = e + 2;
                while q < n && q < e + 10 && b[q].is_ascii_hexdigit() {
                    q += 1;
                }
                if q == e + 2 {
                    return e + 1;
                }
                q - e
            }
            b'c' if b.get(e + 2).is_some_and(u8::is_ascii_lowercase) => 3,
            _ => return e + 1,
        };
        self.put(e, e + len, k::ESCAPE)
    }
}

/// A level's fill and flags for context `c` (not a here-document's word) over a level whose
/// are `pm`: its own fill (its twin inside an option) or the one under it; an option's flag
/// kept and set by [`OPTION`], a backtick's kept by the contexts a backtick ends and set by
/// [`TICK`].
#[inline(always)]
fn derived(pm: u32, c: u8) -> u32 {
    let pfl = (pm >> 16) as u8;
    let info = INFO[usize::from(c)];
    let mut fl = pfl & TW;
    if c == OPTION {
        fl |= TW;
    }
    if c == TICK || info & I_KEEPS_BT != 0 && pfl & BT != 0 {
        fl |= BT;
    }
    let own = info as u16;
    let fill = if own != 0 { twin(own, fl) } else { pm as u16 };
    u32::from(fill) | u32::from(fl) << 16
}

/// Whether `let`'s word ends at `e` (`\blet\b`).
#[inline(always)]
fn breaks_word(b: &[u8], e: usize) -> bool {
    e >= b.len() || !WORD[usize::from(b[e])]
}

/// `kind`, or its twin where `fl` is inside an option.
#[inline(always)]
fn twin(kind: u16, fl: u8) -> u16 {
    if fl & TW != 0 && kind != PLAIN && is_variable(kind) {
        kind + TWIN
    } else {
        kind
    }
}

// what `expand` takes
const X_STR: u8 = 1;
const X_PAT: u8 = 2;
const X_BRACE: u8 = 4;
const X_TILDE: u8 = 8;
const X_JOB: u8 = 16;
/// `$(` tried before `$((` (a case's patterns: bat includes them in that order).
const X_CMD_FIRST: u8 = 32;
/// Parameters, arithmetic and substitutions only (in `"…"`, a here-document).
const X_EXP: u8 = 0;
/// bat's `expansion-and-string`.
const X_ALL: u8 = X_STR | X_PAT | X_BRACE | X_TILDE | X_JOB;

// --- the fast loop (R1′) ---------------------------------------------------------------------

impl<K: Sink> Scan<'_, '_, K> {
    /// Whether a line may start on [`Scan::fast`]: only holders open (a command may start), none
    /// with a fill or a flag.
    #[inline(always)]
    fn fast_ok(&self) -> bool {
        self.text == PLAIN
            && self.fl == 0
            && matches!(
                self.cur,
                ROOT | EXPCMD | SUBSHELL | BRACES | FUNCBODY | CASECMDS | PROC
            )
    }

    /// The floor probe's loop (X-093's R1′): a line's commonest tokens, one turn a token, the
    /// command's position in a local (`pos`: 0 at the holder, else the context the stack loop
    /// would have on top: [`ARGS`], [`EOO`], a builtin's [`BARGS`]); nothing pushed, so a line it
    /// takes whole leaves the stack as it was. What it doesn't take (a here-document, `${…}`, a
    /// definition…) it hands to [`Scan::run`] at that token, the contexts open there pushed: its
    /// spans are the stack loop's, which the digest holds.
    #[inline(always)]
    fn fast(&mut self, p: usize) -> Option<usize> {
        let r = self.fast_loop(p);
        if r.is_none() && self.text != PLAIN {
            // an assignment's value to the line's end
            self.fill(self.b.len());
            self.text = PLAIN;
        }
        r
    }

    #[inline(always)]
    fn fast_loop(&mut self, mut p: usize) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let holder = self.cur;
        // the command's position: 0 at the holder, else the context the stack loop would have on
        // top (`pos`) over `base` (0, or [`MORE`] under a declaration's names and values, or
        // [`ARGS`] under an option); a name's and a value's text the top's fill while they last
        let mut pos: u8 = 0;
        let mut base: u8 = 0;
        while p < n {
            let c = b[p];
            match pos {
                0 => match c {
                    b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {
                        p += 1;
                        while p < n && is_space(b[p]) {
                            p += 1;
                        }
                    }
                    b'#' if p == 0 || is_space(b[p - 1]) => {
                        self.comment(p);
                        return None;
                    }
                    b';' | b'|' | b'&' => match fast_op(b, p) {
                        Some((e, kind)) => p = self.put(p, e, kind),
                        None => return self.fall(p, 0, 0, 0),
                    },
                    _ if WORD[usize::from(c)] || matches!(c, b'/' | b'.' | b':') => {
                        match self.fast_command(p, holder) {
                            Some((e, at, under)) => {
                                p = e;
                                pos = at;
                                base = under;
                            }
                            None => return self.fall(p, 0, 0, 0),
                        }
                    }
                    _ => return self.fall(p, 0, 0, 0),
                },
                VALUE | VARNAME | MORE | DECL | DECL_EXPORT | DECL_READONLY => {
                    match self.fast_names(p, pos, base) {
                        Ok((q, at, under)) => {
                            p = q;
                            pos = at;
                            base = under;
                        }
                        Err(r) => return r,
                    }
                }
                _ => match c {
                    // a command's arguments ([`ARGS`], [`EOO`], a builtin's [`BARGS`])
                    b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {
                        let mut e = p + 1;
                        while e < n && is_space(b[e]) {
                            e += 1;
                        }
                        if e == n {
                            return None;
                        }
                        let r = match b[e] {
                            b'#' => {
                                self.comment(e);
                                return None;
                            }
                            b'-' => self.fast_dash(p, e, pos),
                            b'+' if pos == BARGS + O_SET => self.fast_dash(p, e, pos),
                            _ if pos >= BARGS && OPTS[usize::from(pos - BARGS)].bare != 0 => {
                                Ok(self.bare_of(e, pos - BARGS).map(|q| (q, pos)))
                            }
                            _ => Ok(None),
                        };
                        match r {
                            Ok(Some((q, at))) => {
                                p = q;
                                pos = at;
                            }
                            Ok(None) => p = e,
                            Err((q, at)) => return self.fall(q, ARGS, at, 0),
                        }
                    }
                    b';' | b'|' | b'&' => match fast_op(b, p) {
                        Some((e, kind)) => {
                            p = self.put(p, e, kind);
                            pos = 0;
                        }
                        None => return self.fall(p, 0, pos, 0),
                    },
                    b'"' => match self.fast_dq(p) {
                        Ok(e) => p = e,
                        Err(q) => return self.fall(q, 0, pos, DQ),
                    },
                    b'\'' => match self.fast_sq(p) {
                        Ok(e) => p = e,
                        Err(q) => return self.fall(q, 0, pos, SQ),
                    },
                    b'$' => match self.fast_dollar(p, X_ALL, PLAIN) {
                        Ok(Some(e)) => p = e,
                        Ok(None) => p = plain_arg(b, p),
                        Err(()) => return self.fall(p, 0, pos, 0),
                    },
                    b'*' | b'?' => {
                        if b.get(p + 1) == Some(&b'(') {
                            return self.fall(p, 0, pos, 0);
                        }
                        p = self.put(p, p + 1, k::QUANTIFIER);
                    }
                    b'+' | b'@' | b'!' if b.get(p + 1) == Some(&b'(') => {
                        return self.fall(p, 0, pos, 0);
                    }
                    b'[' if self.bracket_after(p) => return self.fall(p, 0, pos, 0),
                    b'~' => p = self.put(p, p + 1, k::TILDE),
                    b')' | b'}' | b'`' | b'\\' | b'<' | b'>' | b'{' | b'%' => {
                        return self.fall(p, 0, pos, 0);
                    }
                    _ => p = plain_arg(b, p),
                },
            }
        }
        None
    }

    /// The fast loop's turns in a declaration's or an assignment's names and values (out of the
    /// hot loop: [`VALUE`], [`VARNAME`], [`MORE`], [`DECL`]…): the place, position and base after
    /// them; `Err` with the loop's result where it ends here.
    #[inline(never)]
    fn fast_names(
        &mut self,
        mut p: usize,
        mut pos: u8,
        mut base: u8,
    ) -> Result<(usize, u8, u8), Option<usize>> {
        let b = self.b;
        let n = b.len();
        while p < n {
            let c = b[p];
            match pos {
                VALUE => match c {
                    // an assignment's unquoted value
                    _ if META[usize::from(c)] || c == b'`' => {
                        self.fill(p);
                        self.text = PLAIN;
                        pos = base;
                        base = 0;
                    }
                    b'"' => match self.fast_dq(p) {
                        Ok(e) => p = e,
                        Err(q) => return Err(self.fall(q, base, pos, DQ)),
                    },
                    b'\'' => match self.fast_sq(p) {
                        Ok(e) => p = e,
                        Err(q) => return Err(self.fall(q, base, pos, SQ)),
                    },
                    b'$' => match self.fast_dollar(p, X_ALL, PLAIN) {
                        Ok(Some(e)) => p = e,
                        Ok(None) => p = plain_value(b, p),
                        Err(()) => return Err(self.fall(p, base, pos, 0)),
                    },
                    b'*' | b'?' => {
                        if b.get(p + 1) == Some(&b'(') {
                            return Err(self.fall(p, base, pos, 0));
                        }
                        p = self.put(p, p + 1, k::QUANTIFIER);
                    }
                    b'+' | b'@' | b'!' if b.get(p + 1) == Some(&b'(') => {
                        return Err(self.fall(p, base, pos, 0));
                    }
                    b'[' if self.bracket_after(p) => return Err(self.fall(p, base, pos, 0)),
                    b'~' => p = self.put(p, p + 1, k::TILDE),
                    b'\\' | b'{' | b'%' => return Err(self.fall(p, base, pos, 0)),
                    _ => p = plain_value(b, p),
                },
                VARNAME => {
                    // a declaration's name ([`Scan::varname`]'s), its operator, its value
                    let next = b.get(p + 1).copied();
                    let op = c == b'=' || matches!(c, b'+' | b'-' | b'?') && next == Some(b'=');
                    if op || is_space(c) || META[usize::from(c)] || c == b'`' {
                        let e = if c == b'=' {
                            p + 1
                        } else if op {
                            p + 2
                        } else {
                            p
                        };
                        match b.get(e) {
                            Some(b'(') => return Err(self.fall(p, base, pos, 0)),
                            Some(b'&' | b'`') | None => {
                                if e > p {
                                    self.put(p, e, k::ASSIGN);
                                }
                                self.fill(e);
                                self.text = PLAIN;
                                pos = base;
                                base = 0;
                            }
                            _ if e > p => {
                                self.put(p, e, k::ASSIGN);
                                self.text = k::UNQUOTED;
                                pos = VALUE;
                            }
                            _ => {
                                self.fill(e);
                                self.text = PLAIN;
                                pos = base;
                                base = 0;
                            }
                        }
                        p = e;
                    } else {
                        match c {
                            b'[' | b'"' | b'\'' | b'\\' => return Err(self.fall(p, base, pos, 0)),
                            b'$' if matches!(next, Some(b'\'' | b'"')) => {
                                return Err(self.fall(p, base, pos, 0));
                            }
                            _ => {
                                let mut e = p + 1;
                                while e < n
                                    && !matches!(
                                        b[e],
                                        b'[' | b'"'
                                            | b'\''
                                            | b'\\'
                                            | b'='
                                            | b'+'
                                            | b'-'
                                            | b'?'
                                            | b'$'
                                    )
                                    && !is_space(b[e])
                                    && !META[usize::from(b[e])]
                                    && b[e] != b'`'
                                {
                                    e += 1;
                                }
                                p = e;
                            }
                        }
                    }
                }
                MORE => match c {
                    // after a declaration's name: more names (bat's `vardef-maybe-more`)
                    b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {
                        let mut e = p + 1;
                        while e < n && is_space(b[e]) {
                            e += 1;
                        }
                        if e == n {
                            return Err(None);
                        }
                        match b[e] {
                            b'#' => pos = 0,
                            b'-' => {
                                if let Ok(Some((q, at))) = self.fast_dash(p, e, pos) {
                                    p = q;
                                    pos = at;
                                    continue;
                                }
                            }
                            _ => {}
                        }
                        p = e;
                    }
                    b';' | b'|' | b'&' => match fast_op(b, p) {
                        Some((e, kind)) => {
                            p = self.put(p, e, kind);
                            pos = 0;
                        }
                        None => return Err(self.fall(p, 0, pos, 0)),
                    },
                    b'"' => match self.fast_dq(p) {
                        Ok(e) => p = e,
                        Err(q) => return Err(self.fall(q, 0, pos, DQ)),
                    },
                    b'\'' => match self.fast_sq(p) {
                        Ok(e) => p = e,
                        Err(q) => return Err(self.fall(q, 0, pos, SQ)),
                    },
                    b'$' => match self.fast_dollar(p, X_ALL, PLAIN) {
                        Ok(Some(e)) => p = e,
                        Ok(None) => {
                            self.fill(p);
                            self.text = k::VAR_ASSIGN;
                            pos = VARNAME;
                            base = MORE;
                        }
                        Err(()) => return Err(self.fall(p, 0, pos, 0)),
                    },
                    b'*' | b'?' => {
                        if b.get(p + 1) == Some(&b'(') {
                            return Err(self.fall(p, 0, pos, 0));
                        }
                        p = self.put(p, p + 1, k::QUANTIFIER);
                    }
                    b'+' | b'@' | b'!' if b.get(p + 1) == Some(&b'(') => {
                        return Err(self.fall(p, 0, pos, 0));
                    }
                    b'[' if self.bracket_after(p) => return Err(self.fall(p, 0, pos, 0)),
                    b'~' => p = self.put(p, p + 1, k::TILDE),
                    b')' | b'}' | b'`' | b'\\' | b'<' | b'>' | b'{' | b'%' => {
                        return Err(self.fall(p, 0, pos, 0));
                    }
                    _ => {
                        self.fill(p);
                        self.text = k::VAR_ASSIGN;
                        pos = VARNAME;
                        base = MORE;
                    }
                },
                DECL | DECL_EXPORT | DECL_READONLY => {
                    // a declaration's options ([`Scan::decl`]'s), then its first name
                    let mut q = p;
                    while q < n && is_space(b[q]) {
                        q += 1;
                    }
                    if q < n && b[q] == b'-' {
                        let set: &[u8] = match pos {
                            DECL => b"aAfFgilnrtux",
                            DECL_EXPORT => b"fn",
                            _ => b"aAf",
                        };
                        let mut e = q + 1;
                        while e < n && set.contains(&b[e]) {
                            e += 1;
                        }
                        if e == q + 1 && b.get(e) == Some(&b'p') {
                            e += 1;
                        }
                        if e > q + 1 {
                            self.put(q, q + 1, k::OPTION_MARK);
                            p = self.put(q + 1, e, k::OPTION);
                            continue;
                        }
                    }
                    p = q;
                    self.fill(p);
                    self.text = k::VAR_ASSIGN;
                    pos = VARNAME;
                    base = MORE;
                }
                _ => break,
            }
        }
        Ok((p, pos, base))
    }

    /// The stack loop's contexts at the fast loop's position pushed, `base`, `pos` and `inner`
    /// over them (each where not 0; at the end of the spans so far, so that text not yet filled
    /// is the innermost one's): the place to resume at.
    #[cold]
    #[inline(never)]
    fn fall(&mut self, p: usize, base: u8, pos: u8, inner: u8) -> Option<usize> {
        // a name's or a value's fill is its context's again, pushed below
        self.text = PLAIN;
        if FALLS {
            let w: String = self.b[p..].iter().take(12).map(|&c| c as char).collect();
            eprintln!("FALL {pos} {inner} {} {w:?}", self.b.len() - p);
        }
        let at = self.at;
        for c in [base, pos, inner] {
            if c != 0 {
                self.push(at, c);
            }
        }
        Some(p)
    }

    /// A command's first word at `p` (a word byte) when the fast loop takes it: a word of word
    /// bytes ending at a blank or the line's end that is a name, a keyword a command or
    /// arguments follow, or a builtin; its end and the position after it. `None` for the rest
    /// (definitions, assignments, the words with contexts of their own, a comment after a name).
    #[inline(always)]
    fn fast_command(&mut self, p: usize, holder: u8) -> Option<(usize, u8, u8)> {
        let b = self.b;
        let n = b.len();
        let mut e = p;
        while e < n && WORD[usize::from(b[e])] {
            e += 1;
        }
        if e < n && (b[e] == b'=' || b[e] == b'+' && b.get(e + 1) == Some(&b'=')) {
            // `name=`, `name+=` ([`Scan::command`]'s): the name, the operator, the value
            let v = if b[e] == b'=' { e + 1 } else { e + 2 };
            if v > e + 1
                && matches!(
                    lookup(b, p, e).0,
                    W_DECLARE | W_EXPORT | W_READONLY | W_ALIAS | W_COPROC
                )
            {
                return None;
            }
            match b.get(v) {
                Some(b'(') => return None,
                Some(b'&' | b'`') | None => {
                    self.put(p, e, k::VAR_ASSIGN);
                    return Some((self.put(e, v, k::ASSIGN), 0, 0));
                }
                _ => {
                    self.put(p, e, k::VAR_ASSIGN);
                    self.put(e, v, k::ASSIGN);
                    self.text = k::UNQUOTED;
                    return Some((v, VALUE, 0));
                }
            }
        }
        let c = b[p];
        if e == p && c != b'/' && builtin_ends(b, p + 1) {
            // `:` and `.`, the builtins, before a blank ([`Scan::command`]'s simple word)
            if p + 1 < n && !matches!(b[p + 1], b' ' | b'\t' | b'\r') || self.parens_after(p + 1) {
                return None;
            }
            let (_, kind, o) = word(&b[p..p + 1]);
            return Some((self.put(p, p + 1, kind), BARGS + o, 0));
        }
        if e < n && matches!(b[e], 0x0b | 0x0c) {
            // [`Scan::command`]'s blank, not a name's stop
            return None;
        }
        let simple = e > p && (e == n || matches!(b[e], b' ' | b'\t' | b'\r'));
        let (what, kind, o) = if e > p {
            lookup(b, p, e)
        } else {
            (W_NONE, 0, 0)
        };
        if simple {
            if !c.is_ascii_digit() && self.parens_after(e) {
                return None;
            }
        } else {
            // a word any other byte ends: a name only ([`Scan::command`]'s definition,
            // assignment and redirection read first)
            if what != W_NONE || self.defines_at(p) || self.assigns(p) {
                return None;
            }
            let mut d = p;
            while d < n && b[d].is_ascii_digit() {
                d += 1;
            }
            if d < n && matches!(b[d], b'<' | b'>' | b'&') {
                return None;
            }
        }
        match what {
            W_NONE => {}
            W_BUILTIN => {
                self.put(p, e, kind);
                return Some((e, BARGS + o, 0));
            }
            W_DECLARE => return Some((self.put(p, e, kind), DECL, MORE)),
            W_EXPORT => return Some((self.put(p, e, kind), DECL_EXPORT, MORE)),
            W_READONLY => return Some((self.put(p, e, kind), DECL_READONLY, MORE)),
            _ if !self.word_starts(p) => return None,
            W_IF => return Some((self.put(p, e, kind), 0, 0)),
            W_ARGS => return Some((self.put(p, e, kind), ARGS, 0)),
            W_ESAC if holder != CASECMDS => return Some((self.put(p, e, kind), 0, 0)),
            _ => return None,
        }
        // a name, up to where a name's plain run ends
        let mut q = e;
        while q < n && !NAME_STOP[usize::from(b[q])] {
            q += 1;
        }
        if q == p
            || q < n
                && (matches!(b[q], b'"' | b'\'' | b'$' | b'~' | b'%' | b'\\')
                    || is_space(b[q]) && self.comment_follows(q))
        {
            return None;
        }
        Some((self.put(p, q, COMMAND), ARGS, 0))
    }

    /// `-` (or `set`'s `+`) at `e` after the blank run from `p`, in the arguments at `pos`:
    /// [`Scan::dash`]'s, an option's name taken whole. `Err` with the place to resume at and
    /// the contexts to fall back with there.
    #[inline(always)]
    fn fast_dash(
        &mut self,
        p: usize,
        e: usize,
        pos: u8,
    ) -> Result<Option<(usize, u8)>, (usize, u8)> {
        let b = self.b;
        let n = b.len();
        if pos == EOO {
            return Ok(None);
        }
        if b[e] == b'-' && b.get(e + 1) == Some(&b'-') && (e + 2 >= n || is_space(b[e + 2])) {
            self.put(p, e + 2, k::END_OF_OPTIONS);
            return Ok(Some((e + 2, EOO)));
        }
        if pos == ARGS {
            let mut q = e + 1;
            if b.get(q) == Some(&b'-') {
                q += 1;
            }
            if !(q < n && (WORD[usize::from(b[q])] || b[q] == b'$')) {
                return Ok(None);
            }
            self.put(p, q, k::OPTION_MARK);
            // the option's name: word bytes and `-`, then `=`, a blank or another byte
            let mut r = q;
            while r < n && (WORD[usize::from(b[r])] || b[r] == b'-') {
                r += 1;
            }
            if r > q {
                self.put(q, r, k::OPTION);
            }
            if r < n && (b[r] == b'$' || is_space(b[r]) && self.comment_follows(r)) {
                return Err((r, OPTION));
            }
            if r < n && b[r] == b'=' {
                return Ok(Some((self.put(r, r + 1, k::OPTION_ASSIGN), ARGS)));
            }
            return Ok(Some((r, ARGS)));
        }
        if pos < BARGS {
            return Ok(None);
        }
        let o = pos - BARGS;
        let spec = &OPTS[usize::from(o)];
        if spec.dash == 0 {
            return Ok(None);
        }
        let mut q = e + 1;
        if o == O_SET && q < n && b[q] == b'o' && option_ends(b, q + 1) {
            q += 1;
        } else if spec.many {
            while q < n && in_set(spec.dash, b[q]) {
                q += 1;
            }
            if q == e + 1 || !option_ends(b, q) {
                return Ok(None);
            }
        } else {
            if q >= n || !in_set(spec.dash, b[q]) || !option_ends(b, q + 1) {
                return Ok(None);
            }
            q += 1;
        }
        if b[e] == b'+' && o != O_SET {
            return Ok(None);
        }
        self.put(e, e + 1, k::OPTION_MARK);
        Ok(Some((self.put(e + 1, q, k::OPTION), pos)))
    }

    /// A builtin's bare option at `e` ([`Scan::bare`]'s), its option set `o`.
    #[inline(always)]
    fn bare_of(&mut self, e: usize, o: u8) -> Option<usize> {
        let b = self.b;
        let n = b.len();
        let spec = &OPTS[usize::from(o)];
        let mut q = e;
        while q < n && in_set(spec.bare, b[q]) {
            q += 1;
        }
        if q == e || !option_ends(b, q) {
            return None;
        }
        Some(self.put(e, q, k::OPTION))
    }

    /// `'…'` at `p` closed on its line: its end; `Err` with the place to resume at inside it
    /// where it goes on past the line.
    #[inline(always)]
    fn fast_sq(&mut self, p: usize) -> Result<usize, usize> {
        let b = self.b;
        let n = b.len();
        self.put(p, p + 1, k::STR_BEGIN);
        let e = find_in(b, p + 1, n, *b"'");
        if e >= n {
            return Err(p + 1);
        }
        if e > p + 1 {
            self.put(p + 1, e, k::STR_SINGLE);
        }
        Ok(self.put(e, e + 1, k::STR_END))
    }

    /// `"…"` at `p` closed on its line, with parameters and escapes in it: its end. `Err` with
    /// the place to resume at inside it where something else starts (a substitution, `${`,
    /// the line's end).
    #[inline(always)]
    fn fast_dq(&mut self, p: usize) -> Result<usize, usize> {
        let b = self.b;
        let n = b.len();
        self.put(p, p + 1, k::STR_BEGIN);
        let mut q = p + 1;
        loop {
            let e = find_in(b, q, n, *b"\"$`\\");
            if e >= n {
                return Err(q);
            }
            match b[e] {
                b'"' => {
                    if e > self.at {
                        self.put(self.at, e, k::STR_DOUBLE);
                    }
                    return Ok(self.put(e, e + 1, k::STR_END));
                }
                b'\\' => {
                    if e + 1 == n {
                        return Err(e);
                    }
                    if matches!(b[e + 1], b'$' | b'`' | b'"' | b'\\') {
                        if e > self.at {
                            self.put(self.at, e, k::STR_DOUBLE);
                        }
                        q = self.put(e, e + 2, k::ESCAPE);
                    } else {
                        q = e + 1;
                    }
                }
                b'$' => match self.fast_dollar(e, X_EXP, k::STR_DOUBLE) {
                    Ok(Some(r)) => q = r,
                    Ok(None) => q = e + 1,
                    Err(()) => return Err(e),
                },
                _ => return Err(e),
            }
        }
    }

    /// `$` at `p` ([`Scan::dollar`]'s) when it is a parameter: its end; `Ok(None)` for a `$`
    /// alone; `Err` where a context starts (`${`, `$(`, `$'`, `$"`). The text before it is
    /// `fill`'s, put first.
    #[inline(always)]
    fn fast_dollar(&mut self, p: usize, what: u8, fill: u16) -> Result<Option<usize>, ()> {
        let b = self.b;
        let n = b.len();
        let Some(&c) = b.get(p + 1) else {
            return Ok(None);
        };
        let (e, kind) = match c {
            b'{' | b'(' => return Err(()),
            b'\'' | b'"' if what & X_STR != 0 => return Err(()),
            b'0'..=b'9' => (p + 2, k::READWRITE),
            b'$' | b'#' | b'@' | b'!' | b'~' | b'*' | b'?' | b'_' | b'-'
                if p + 2 >= n || !WORD[usize::from(b[p + 2])] =>
            {
                (p + 2, k::LANG_VAR)
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                let mut e = p + 2;
                while e < n && WORD[usize::from(b[e])] {
                    e += 1;
                }
                (e, k::READWRITE)
            }
            _ => return Ok(None),
        };
        if fill != PLAIN && p > self.at {
            self.put(self.at, p, fill);
        }
        self.put(p, p + 1, k::VAR_MARK);
        Ok(Some(self.put(p + 1, e, kind)))
    }
}

/// Where ada-parse is built with `ADA_PARSE_X093_FALLS` set: each hand-over from the fast loop
/// to the stack loop, and each line that can't start on the fast loop, to stderr (a census of
/// what the fast loop doesn't take).
const FALLS: bool = option_env!("ADA_PARSE_X093_FALLS").is_some();

/// `;`, `|`, `&` at `p` ending a command (cmd-post's): its end and kind; `None` for `;;`, `;&`
/// and `&>`.
#[inline(always)]
fn fast_op(b: &[u8], p: usize) -> Option<(usize, u16)> {
    let next = b.get(p + 1).copied();
    Some(match b[p] {
        b';' if !matches!(next, Some(b';' | b'&')) => (p + 1, k::CONTINUE),
        b'|' if next == Some(b'|') => (p + 2, k::OR),
        b'|' => (p + 1, k::PIPE),
        b'&' if next == Some(b'&') => (p + 2, k::AND),
        b'&' if next != Some(b'>') => (p + 1, k::JOB),
        _ => return None,
    })
}

/// An unquoted value's plain run from `p`.
#[inline(always)]
fn plain_value(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p + 1;
    while e < n && !VALUE_STOP[usize::from(b[e])] {
        e += 1;
    }
    e
}

/// An argument's plain run from `p`.
#[inline(always)]
fn plain_arg(b: &[u8], p: usize) -> usize {
    let n = b.len();
    let mut e = p + 1;
    while e < n && !ARG_STOP[usize::from(b[e])] {
        e += 1;
    }
    e
}

// --- the entry points -------------------------------------------------------------------------

/// Lex `b`, a line of bash without its `\n`, from `state`: the span API's scan, the span sink's
/// writes inline (ada doesn't call it: its path is [`line_runs`]').
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    if state.depth == 0 && plain_line(b, out) {
        return;
    }
    let mut s = Scan::fresh(b, out, Spans::new());
    s.load(state);
    s.k.open(s.out, b.len() / 4 + 16);
    let same = s.lex_line();
    s.k.close(s.out);
    if !same {
        *state = s.keep();
    }
}

/// [`line()`]'s spans, pushed to `out` after those there, from the run sink's scan (X-094):
/// Makefile's recipes lay theirs around them. ada holds one copy of bash's scan, the run sink's,
/// a write a call ([`Sink::merge_call`]); its runs by [`Kinds`], whose rule for a kind is the
/// kind, are the spans.
#[inline(never)]
pub(crate) fn line_spans(b: &[u8], state: &mut State, out: &mut Out) {
    if state.depth == 0 && plain_line(b, out) {
        return;
    }
    thread_local! {
        static RUNS: std::cell::RefCell<Vec<Run>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    RUNS.with_borrow_mut(|runs| {
        runs.clear();
        line_runs(b, state, out, &mut Kinds, runs);
        spans_of(runs, out);
    });
}

/// Rules whose rule for a kind is the kind: runs that are the spans, joined where a kind meets
/// itself as the span sink joins them, the gaps [`Rule::NONE`]'s, which no kind is.
struct Kinds;

/// [`Kinds`]' table: every kind bash writes, its twins with them.
static KINDS_AS_RULES: [Rule; 0x400] = {
    let mut t = [Rule(0); 0x400];
    let mut i = 0;
    while i < t.len() {
        t[i] = Rule(i as u16);
        i += 1;
    }
    t
};

impl Rules for Kinds {
    fn table(&self) -> &[Rule] {
        &KINDS_AS_RULES
    }

    fn resolve(&mut self, kind: u16) -> Rule {
        Rule(kind)
    }
}

/// A line's runs by [`Kinds`] as its spans, pushed to `out`.
#[inline(always)]
fn spans_of(runs: &[Run], out: &mut Out) {
    let mut at = 0;
    for r in runs {
        if r.rule.0 != Rule::NONE {
            out.0.push(crate::Span {
                start: at,
                end: r.end,
                kind: r.rule.0,
            });
        }
        at = r.end;
    }
}

/// [`line()`]'s runs by theme rule pushed to `runs`.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, out, Runs::new(rules, runs));
    s.load(state);
    if !s.lex_line() {
        *state = s.keep();
    }
    s.k.end_line(b.len());
}

/// A line with nothing open before it that is blank or a comment (a third of a script's
/// lines): its spans pushed without a scan, the state as it was; `false` for any other line.
#[inline(always)]
fn plain_line(b: &[u8], out: &mut Out) -> bool {
    let n = b.len();
    let mut i = 0;
    while i < n && is_space(b[i]) {
        i += 1;
    }
    if i == n {
        return true;
    }
    if b[i] != b'#' {
        return false;
    }
    out.0.push(crate::Span {
        start: i as u32,
        end: i as u32 + 1,
        kind: k::COMMENT_BEGIN,
    });
    if i + 1 < n {
        out.0.push(crate::Span {
            start: i as u32 + 1,
            end: n as u32,
            kind: k::COMMENT,
        });
    }
    true
}

/// A block's lines on one scan opened from `state` ([`crate::Carry`]), the span API's.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", &mut out, Spans::new());
    s.load(state);
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = s.keep();
    rest
}

impl crate::Carry for Scan<'_, '_, Spans> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.k.start_line();
        // SAFETY: the line is set back to an empty one before `b`'s borrow ends
        self.carry(unsafe { crate::line_for(b) });
        self.k.close(self.out);
        self.out.0
    }
}

/// [`block`]'s runs by theme rule pushed to `runs`, the lines' loop compiled with the scan.
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", &mut out, Runs::new(rules, runs));
    s.load(state);
    let rest = lines.pass(&mut s);
    *state = s.keep();
    rest
}

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    #[inline(never)]
    fn next(&mut self, b: &[u8]) -> usize {
        self.k.start_line();
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

impl<'b, K: Sink> Scan<'b, '_, K> {
    /// `b`, the block's next line, lexed on this scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.b = b;
        self.lex_line();
        self.b = b"";
    }
}

#[cfg(test)]
mod tests {
    use crate::{Lang, State};

    /// Each line's spans as (text, scopes), the lines lexed one at a time.
    fn spans(lines: &[&str]) -> Vec<Vec<(String, String)>> {
        let mut st = State::default();
        let mut v = Vec::new();
        lines
            .iter()
            .map(|l| {
                v.clear();
                super::line(l.as_bytes(), &mut st, &mut crate::Out::new(&mut v));
                v.iter()
                    .map(|s| {
                        (
                            l[s.start as usize..s.end as usize].to_owned(),
                            Lang::Bash.scopes(s.kind).join(" "),
                        )
                    })
                    .collect()
            })
            .collect()
    }

    /// A line's tokens as (text, leaf scope), through `lex` (the switch's scanner).
    fn leaves(line: &str) -> Vec<(String, String)> {
        let (mut st, mut v) = (State::default(), Vec::new());
        crate::lex(Lang::Bash, line, &mut st, &mut v);
        v.iter()
            .map(|s| {
                let sc = Lang::Bash.scopes(s.kind);
                (
                    line[s.start as usize..s.end as usize].to_owned(),
                    sc.last().copied().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|&(t, s)| (t.to_owned(), s.to_owned()))
            .collect()
    }

    /// The fast loop's corners (X-093's R1′), each as the stack loop reads it: a declaration's
    /// blanks aren't its name's, an option's `=` is the option's and the value after it an
    /// argument's, a value runs to a blank.
    #[test]
    fn the_fast_loop_reads_as_the_stack_loop() {
        let assign = "variable.other.readwrite.assignment.shell";
        let command = *Lang::Bash.scopes(super::COMMAND).last().unwrap();
        assert_eq!(
            leaves("export  A=1 B"),
            pairs(&[
                ("export", "storage.modifier.shell"),
                ("A", assign),
                ("=", "keyword.operator.assignment.shell"),
                ("1", "string.unquoted.shell"),
                ("B", assign),
            ])
        );
        assert_eq!(
            leaves("local -r x=\"$y\"z"),
            pairs(&[
                ("local", "storage.modifier.shell"),
                ("-", "punctuation.definition.parameter.shell"),
                ("r", "variable.parameter.option.shell"),
                ("x", assign),
                ("=", "keyword.operator.assignment.shell"),
                ("\"", "punctuation.definition.string.begin.shell"),
                ("$", "punctuation.definition.variable.shell"),
                ("y", "variable.other.readwrite.shell"),
                ("\"", "punctuation.definition.string.end.shell"),
                ("z", "string.unquoted.shell"),
            ])
        );
        assert_eq!(
            leaves("ls -la --color=$c x"),
            pairs(&[
                ("ls", command),
                (" -", "punctuation.definition.parameter.shell"),
                ("la", "variable.parameter.option.shell"),
                (" --", "punctuation.definition.parameter.shell"),
                ("color", "variable.parameter.option.shell"),
                ("=", "keyword.operator.assignment.option.shell"),
                ("$", "punctuation.definition.variable.shell"),
                ("c", "variable.other.readwrite.shell"),
            ])
        );
        assert_eq!(
            leaves("set -o pipefail"),
            pairs(&[
                ("set", "support.function.set.shell"),
                ("-", "punctuation.definition.parameter.shell"),
                ("o", "variable.parameter.option.shell"),
            ])
        );
    }

    /// A script lexed line by line and as one carried block (every line through the fast loop,
    /// the stack loop, or both) gives the same spans and states.
    #[test]
    fn a_block_lexes_as_its_lines() {
        let text = [
            "#!/bin/bash",
            "f() {",
            "  local x=1 y=\"a $b",
            "c\" z",
            "  echo \"$x\" ${y:-d} $(cmd -v | wc -l) > /dev/null 2>&1",
            "  [[ -f $x ]] && export A=1 || return 1",
            "  case $1 in",
            "    a|b) foo; bar=2 ;;",
            "  esac",
            "  cat <<-EOF",
            "\tbody $x",
            "\tEOF",
            "  ls -la \\",
            "    --color=auto '",
            "  quoted' x=1",
            "}",
            "for i in 1 2; do x+=$i; done",
            // several here-documents on a line, more than the held levels keep (a line keeps
            // 28, five a body): the bodies in the openers' order, the lowest ones cut
            "cat <<A1 <<'A2' <<-A3 <<A4 <<A5 <<A6 <<A7",
            "a1 $x",
            "A1",
            "a2 $x",
            "A2",
            "\ta3",
            "\tA3",
            "A4",
            "A5",
            "A6",
            "A7",
            "echo after",
        ]
        .join("\n");
        let (mut want, mut state, mut spans) = (Vec::new(), State::default(), Vec::new());
        for line in text.split('\n') {
            crate::lex(Lang::Bash, line, &mut state, &mut spans);
            want.push((line.to_owned(), spans.clone(), state));
        }
        for max in [1, 2, usize::MAX] {
            let (mut got, mut state, mut buf) = (Vec::new(), State::default(), String::new());
            let mut rest = Some(&text[..]);
            while let Some(t) = rest {
                let next = crate::lex_lines(
                    Lang::Bash,
                    t,
                    max,
                    &mut state,
                    (&mut spans, &mut buf),
                    |_, _| false,
                    |l, s| got.push((l.to_owned(), s.to_vec())),
                );
                assert_eq!(
                    state,
                    want[got.len() - 1].2,
                    "max {max}, line {}",
                    got.len()
                );
                rest = next.map(|p| &t[p..]);
            }
            let lines: Vec<_> = want
                .iter()
                .map(|(l, s, _)| (l.clone(), s.clone()))
                .collect();
            assert_eq!(got, lines, "max {max}");
        }
    }

    /// The states the fast loop leaves: a line it takes whole leaves the state it was given
    /// (here a function's body), a string it opens goes on to the next line.
    #[test]
    fn the_fast_loop_keeps_the_state() {
        let (mut st, mut v) = (State::default(), Vec::new());
        crate::lex(Lang::Bash, "f() {", &mut st, &mut v);
        let body = st;
        for line in [
            "  local x=1",
            "  echo \"$x\" -n",
            "  a=b cmd arg; other || x",
        ] {
            crate::lex(Lang::Bash, line, &mut st, &mut v);
            assert_eq!(st, body, "{line}");
        }
        crate::lex(Lang::Bash, "  echo \"open", &mut st, &mut v);
        assert_ne!(st, body);
        let got = spans(&["echo \"open", "still\" x"]);
        assert_eq!(
            got[1][0],
            ("still".to_owned(), "string.quoted.double.shell".to_owned())
        );
        crate::lex(Lang::Bash, "closed\" x", &mut st, &mut v);
        assert_eq!(st, body);
        crate::lex(Lang::Bash, "}", &mut st, &mut v);
        assert_eq!(st, State::default());
    }

    /// Lines of the fast loop's constructs repeated, and of the hand-overs from it, lex in
    /// linear time.
    #[test]
    fn long_lines_stay_linear() {
        let lines = [
            "x=".repeat(30000),
            "a=b ".repeat(20000),
            format!("local {}", "a=$b ".repeat(15000)),
            format!("echo {}", "\"$a\" -x --y=z ".repeat(8000)),
            format!("echo \"{}", "$a ".repeat(20000)),
            "${".repeat(20000),
            "$(".repeat(20000),
            "; ".repeat(30000),
            "f ".repeat(30000),
        ];
        let t = std::time::Instant::now();
        let (mut state, mut spans) = (State::default(), Vec::new());
        for line in &lines {
            crate::lex(Lang::Bash, line, &mut state, &mut spans);
        }
        assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
    }

    /// Here-documents opened on one line are read in the openers' order (X-105, a fix over bat,
    /// which pushes the later one over the earlier and reads it first).
    #[test]
    fn here_documents_follow_their_openers_order() {
        let body = "string.unquoted.heredoc.shell";
        let token = "keyword.control.heredoc-token.shell";
        let got = spans(&[
            "cat <<A <<-'B' <<C",
            "line a $x",
            "A",
            "\tline b $x",
            "\tB",
            "C",
            "echo after",
        ]);
        assert!(
            got[1].iter().any(|(t, s)| t == "line a " && s == body),
            "{got:?}"
        );
        assert_eq!(got[2], [("A".to_owned(), token.to_owned())]);
        // B's word was quoted: no expansions in its body
        assert_eq!(got[3], [("\tline b $x".to_owned(), body.to_owned())]);
        assert!(
            got[4].iter().any(|(t, s)| t == "B" && s.ends_with(token)),
            "{got:?}"
        );
        assert_eq!(got[5], [("C".to_owned(), token.to_owned())]);
        assert!(got[6][0].1.contains("support.function.echo"), "{got:?}");
    }

    /// A body whose word's levels the held levels cut (its line keeps 28 levels, the body's
    /// own among them, some of its word's not) ends where it would have been read (X-105: the
    /// word's levels were read below the stack's base, a panic).
    #[test]
    fn a_here_document_cut_by_the_held_levels() {
        for n in 20..30 {
            let open = format!("cat <<A; {}", "{ ".repeat(n));
            let close = "} ".repeat(n);
            let got = spans(&[&open, &close, "body", "A", "echo after"]);
            assert!(
                got[4][0].1.contains("support.function.echo"),
                "{n}: {got:?}"
            );
        }
    }

    #[test]
    fn a_comment_in_a_brace_expansion() {
        let got = spans(&["a {", "  # c"]);
        assert!(
            got[1]
                .iter()
                .any(|(t, s)| t == "#" && s.contains("comment")),
            "{got:?}"
        );
    }
}
