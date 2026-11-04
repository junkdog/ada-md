//! bash by a scanner of ada's own (ADR-40, speed first), in place of the port of syntect's
//! machine that ran it before (`make`, G-003). bat's `Bourne Again Shell (bash)` and its
//! generated `commands-builtin-shell-bash` are the reference: the scanner keeps their contexts,
//! a byte each on a stack, but matches at the place it has reached instead of searching ahead
//! with every pattern in reach. Each context dispatches on the byte there to the few patterns
//! that can start on it, in the grammar's order, which is syntect's leftmost match; where none
//! can, a run of bytes that start nothing is passed whole (anpa's `find_byte` in strings and
//! here-documents), where the port tried dozens of patterns at every token.
//!
//! syntect's machinery is followed where a line's colors depend on it: the prototype (comments,
//! line continuations, escapes) tried first in the contexts that take it; a pop that matches
//! nothing right after a push that matched nothing, at the same place and depth, passed over,
//! else the scan steps a character (a stray `)` at the top level); `$` before the line's `\n`
//! and at its end, and `\s` taking the `\n`.
//!
//! **Kinds are leaves.** The 14 built-in themes key on single scopes, none of them a meta scope
//! shell has, and syntect scores a selector at the outermost scope it matches, so a byte's rule
//! is that of its deepest scope that isn't `meta.*`: a kind carries that scope (with the meta
//! scope bat puts beside it, as `meta.function-call` on a command's name). The one place an
//! outer scope decides is an option: inside `variable.parameter.option`, a `$x` or a command in
//! `$(…)` takes `variable.parameter`'s rule, not `variable`'s, so those kinds have twins that
//! carry the option's scope too ([`TWIN`]). User themes keyed on deeper meta scopes may color
//! differently than on syntect, as the decision on ada-parse accepts.
//!
//! Between lines the stack is kept in [`State`]'s 28 levels, a byte a context; a builtin's
//! arguments keep which builtin, and a here-document its end word, as a hash
//! (FNV-1a), in the bytes after its context.

use std::mem::MaybeUninit;

use anpa::core::parse;
use anpa::findbyte::{eq, find_byte};

use crate::matcher::KEPT as HALF;
use crate::prim::char_len;
use crate::{Out, State};

mod makefile;
mod matchers;
mod scan;

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
}

/// No kind: plain text.
const PLAIN: u16 = u16::MAX;

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

// ---- contexts: bat's, by its names; `_BT` the backtick copies, which also end at a `` ` ``

const MAIN: u8 = 0;
const MAIN_POP_AT_END: u8 = 1;
const COMMENT_BODY: u8 = 2;
const CONTINUED: u8 = 3;
const CMD: u8 = 4;
const CMD_BT: u8 = 5;
const CMD_PAREN: u8 = 6;
const CMD_PAREN_BT: u8 = 7;
const CMD_BRACE: u8 = 8;
const CMD_BRACE_BT: u8 = 9;
const LET: u8 = 10;
const LET_BT: u8 = 11;
const TEST: u8 = 12;
const TEST_BT: u8 = 13;
const TEST_BODY: u8 = 14;
const TEST_BODY_BT: u8 = 15;
const TEST_DOUBLE: u8 = 16;
const CMD_NAME: u8 = 17;
const CMD_NAME_BODY: u8 = 18;
const CMD_ARGS: u8 = 19;
const CMD_ARGS_BT: u8 = 20;
const CMD_ARGS_BODY: u8 = 21;
const CMD_ARGS_BODY_BT: u8 = 22;
const OPTION: u8 = 23;
const OPTION_BT: u8 = 24;
const OPTION_ASSIGN: u8 = 25;
const END_OF_OPTIONS: u8 = 26;
const END_OF_OPTIONS_BT: u8 = 27;
const CMD_POST: u8 = 28;
const CASE_WORD: u8 = 29;
const CASE_BODY: u8 = 30;
const CASE_PAREN: u8 = 31;
const CASE_PATTERNS: u8 = 32;
const CASE_PATTERNS_PAREN: u8 = 33;
const CASE_COMMANDS: u8 = 34;
const FOR_ARGS: u8 = 35;
const FOR_BODY: u8 = 36;
const ARITHMETIC_BODY: u8 = 37;
const EXPRESSION_PAREN: u8 = 38;
const HEX: u8 = 39;
const OCTAL: u8 = 40;
const REGEXP: u8 = 41;
const BRACE_BODY: u8 = 42;
const PARAMETER_START: u8 = 43;
const PARAMETER_POST: u8 = 44;
const PARAMETER_OP: u8 = 45;
const PARAMETER_PATTERN: u8 = 46;
const PARAMETER_COMMON: u8 = 47;
const PARAMETER_SLASH: u8 = 48;
const PARAMETER_SLASH_FIRST: u8 = 49;
const PARAMETER_SUBSTITUTE: u8 = 50;
const PARAMETER_SUBSTRING: u8 = 51;
const PARAMETER_OFFSET: u8 = 52;
const PARAMETER_LENGTH_COLON: u8 = 53;
const PARAMETER_LENGTH: u8 = 54;
const ARITHMETIC_EXPANSION: u8 = 55;
const COMMAND_PARENS: u8 = 56;
const BACKTICKS: u8 = 57;
const EXTGLOB: u8 = 58;
const SET_START: u8 = 59;
const SET_REST: u8 = 60;
const STRING_SINGLE: u8 = 61;
const STRING_DOUBLE: u8 = 62;
const STRING_ANSI_C: u8 = 63;
const STRING_LOCALE: u8 = 64;
const ESCAPED_NEWLINE: u8 = 65;
const ARRAY_BODY: u8 = 66;
const VARDEF_FUNCTION_CALL: u8 = 67;
const VARDEF_MORE: u8 = 68;
const VARDEF_ALIAS_NAME: u8 = 69;
const VARDEF_ALIAS_NAME_BODY: u8 = 70;
const VARDEF_NAME: u8 = 71;
const VARDEF_NAME_BODY: u8 = 72;
const VARDEF_ASSIGN: u8 = 73;
const VARDEF_VALUE: u8 = 74;
const VARDEF_ARRAY: u8 = 75;
const VARDEF_ARRAY_INDEX: u8 = 76;
const VARDEF_ARRAY_EQ: u8 = 77;
const VARDEF_VALUE_WORD: u8 = 78;
const VARDEF_READONLY_OPTIONS: u8 = 79;
const VARDEF_EXPORT_OPTIONS: u8 = 80;
const VARDEF_DECLARE_OPTIONS: u8 = 81;
const VARDEF_ALIAS_OPTIONS: u8 = 82;
const PROCESS_SUBSTITUTION: u8 = 83;
const REDIRECTION_POST: u8 = 84;
const REDIRECTION_TARGET: u8 = 85;
const HEREDOC: u8 = 86;
const HEREDOC_TABS: u8 = 87;
const HEREDOC_LITERAL: u8 = 88;
const HEREDOC_TABS_LITERAL: u8 = 89;
const HEREDOC_PREAMBLE: u8 = 90;
const HEREDOC_LINE: u8 = 91;
const COPROC_BODY: u8 = 92;
const COPROC_NAME_CTX: u8 = 93;
const COPROC_BRACE: u8 = 94;
const COPROC_BRACES: u8 = 95;
const FUNCDEF_NAME: u8 = 96;
const FUNCDEF_NAME_BODY: u8 = 97;
const FUNCDEF_PARENS: u8 = 98;
const FUNCDEF_BODY: u8 = 99;
const FUNCDEF_BODY_BT: u8 = 100;
const BUILTIN_ARGS: u8 = 101;
const BUILTIN_ARGS_BT: u8 = 102;
const BUILTIN_ARGS_BODY: u8 = 103;
const BUILTIN_ARGS_BODY_BT: u8 = 104;
const BUILTIN_END_OF_OPTIONS: u8 = 105;
// in a Makefile: `main` as the recipe, `$(shell …)`, `!=` and `bash -c '…'` embed it, each
// under make patterns of its own ([`makefile`])
const SRC_RECIPE: u8 = 106;
const SRC_CALL: u8 = 107;
const SRC_ASSIGN: u8 = 108;
const SRC_EMBED_SQ: u8 = 109;
const SRC_EMBED_DQ: u8 = 110;
const SRC_EMBED_BT: u8 = 111;
// a comment and a continuation that a recipe's make patterns pushed, which see none of them
const COMMENT_BODY_NW: u8 = 112;
const CONTINUED_NW: u8 = 113;
/// The first of the Makefile grammar's own contexts.
const MK_FIRST: u8 = 114;
const CONTEXTS: usize = MK_FIRST as usize + makefile::COUNT;

/// Per context: whether it takes the prototype, its meta scope's kind and its meta content
/// scope's kind (a non-meta leaf, or [`PLAIN`]).
struct Info {
    proto: bool,
    meta: u16,
    content: u16,
}

const fn info(proto: bool, meta: u16, content: u16) -> Info {
    Info {
        proto,
        meta,
        content,
    }
}

const P: Info = info(true, PLAIN, PLAIN);
const N: Info = info(false, PLAIN, PLAIN);

static INFO: [Info; CONTEXTS] = {
    let mut t = [P; CONTEXTS];
    t[COMMENT_BODY as usize] = info(false, PLAIN, k::COMMENT);
    t[CONTINUED as usize] = N;
    t[CMD_NAME_BODY as usize] = info(true, k::FUNCTION_CALL, PLAIN);
    t[OPTION as usize] = info(true, k::OPTION, PLAIN);
    t[OPTION_BT as usize] = info(true, k::OPTION, PLAIN);
    t[HEX as usize] = info(true, k::HEX, PLAIN);
    t[OCTAL as usize] = info(true, k::OCTAL, PLAIN);
    t[PARAMETER_START as usize] = N;
    t[PARAMETER_POST as usize] = info(true, PLAIN, k::READWRITE);
    t[PARAMETER_PATTERN as usize] = N;
    t[PARAMETER_SLASH_FIRST as usize] = N;
    t[STRING_SINGLE as usize] = info(false, k::STR_SINGLE, PLAIN);
    t[STRING_DOUBLE as usize] = info(false, k::STR_DOUBLE, PLAIN);
    t[STRING_ANSI_C as usize] = info(false, k::STR_ANSI_C, PLAIN);
    t[STRING_LOCALE as usize] = info(false, k::STR_LOCALE, PLAIN);
    t[ESCAPED_NEWLINE as usize] = N;
    t[VARDEF_FUNCTION_CALL as usize] = N;
    t[VARDEF_MORE as usize] = N;
    t[VARDEF_ALIAS_NAME_BODY as usize] = info(false, PLAIN, k::ALIAS_NAME);
    t[VARDEF_NAME_BODY as usize] = info(false, PLAIN, k::VAR_ASSIGN);
    t[VARDEF_ASSIGN as usize] = N;
    t[VARDEF_VALUE as usize] = N;
    t[VARDEF_VALUE_WORD as usize] = info(false, k::UNQUOTED, PLAIN);
    t[HEREDOC as usize] = info(false, k::HEREDOC, PLAIN);
    t[HEREDOC_TABS as usize] = info(false, k::HEREDOC, PLAIN);
    t[HEREDOC_LITERAL as usize] = info(false, k::HEREDOC, PLAIN);
    t[HEREDOC_TABS_LITERAL as usize] = info(false, k::HEREDOC, PLAIN);
    t[COPROC_NAME_CTX as usize] = info(true, PLAIN, k::COPROC_NAME);
    t[FUNCDEF_NAME_BODY as usize] = info(true, PLAIN, k::FUNCTION_NAME);
    t[COMMENT_BODY_NW as usize] = info(false, PLAIN, k::COMMENT);
    t[CONTINUED_NW as usize] = N;
    makefile::info(t)
};

/// Per context, how a frame's [`Frame::flags`] come from the one below: the bits it keeps, and
/// the bits it sets. A shell context keeps its embedding; one that a Makefile embeds the shell
/// with sets its own; a make context, and what a recipe's make patterns push, has none.
static FLAGS: [(u8, u8); CONTEXTS] = {
    let mut t = [(0xff, 0); CONTEXTS];
    let twins = IN_OPTION | IN_PARAM;
    t[OPTION as usize] = (0xff, IN_OPTION);
    t[OPTION_BT as usize] = (0xff, IN_OPTION);
    t[COMMENT_BODY_NW as usize] = (twins, 0);
    t[CONTINUED_NW as usize] = (twins, 0);
    let mut c = MK_FIRST as usize;
    while c < CONTEXTS {
        t[c] = (twins, 0);
        c += 1;
    }
    t[makefile::SUBSTITUTION_PAREN as usize] = (twins, IN_PARAM);
    t[makefile::SUBSTITUTION_BRACE as usize] = (twins, IN_PARAM);
    let mut src = SRC_RECIPE;
    while src <= SRC_EMBED_BT {
        t[src as usize] = (twins, makefile::with_of(src));
        src += 1;
    }
    t
};

/// Whether a context keeps an argument between lines: a builtin's index, or a
/// here-document's end word.
fn has_arg(ctx: u8) -> Option<usize> {
    match ctx {
        BUILTIN_ARGS | BUILTIN_ARGS_BT | BUILTIN_ARGS_BODY | BUILTIN_ARGS_BODY_BT => Some(1),
        HEREDOC | HEREDOC_TABS | HEREDOC_LITERAL | HEREDOC_TABS_LITERAL => Some(4),
        _ => None,
    }
}

/// A builtin: its name, and its options as `((PREFIX)(?:[ONE]|[MANY]+)|[BARE]+)`.
struct Builtin {
    // the table [`builtin`]'s `match` follows, checked in a test
    #[cfg_attr(not(test), allow(dead_code))]
    name: &'static [u8],
    prefix: &'static [u8],
    one: &'static [u8],
    many: &'static [u8],
    bare: &'static [u8],
}

const fn b(
    name: &'static [u8],
    one: &'static [u8],
    many: &'static [u8],
    bare: &'static [u8],
) -> Builtin {
    Builtin {
        name,
        prefix: b"-",
        one,
        many,
        bare,
    }
}

/// In the grammar's order, which a name's match follows; `set` takes `+` too. Whether a
/// builtin takes `--` doesn't matter: the arguments' own `--` comes first.
static BUILTINS: [Builtin; 40] = [
    Builtin {
        name: b"set",
        prefix: b"-+",
        one: b"o",
        many: b"abefhkmnptuvxBCEHPT",
        bare: b"",
    },
    b(b"help", b"", b"dms", b""),
    b(b"mapfile", b"dnOstuCc", b"", b""),
    b(b"getopts", b"", b"", b""),
    b(b"cd", b"LPe@", b"", b""),
    b(b"suspend", b"f", b"", b""),
    b(b":", b"", b"", b""),
    b(b"printf", b"v", b"", b""),
    b(b"hash", b"rpl", b"", b"dt"),
    b(b"ulimit", b"", b"HSabcdefiklmnpqrstuvxPT", b""),
    b(b"umask", b"pS", b"", b""),
    b(b"source", b"", b"", b""),
    b(b"builtin", b"", b"", b""),
    b(b"exit", b"", b"", b""),
    b(b"type", b"", b"afptP", b""),
    b(b"bg", b"", b"", b""),
    b(b"return", b"", b"", b""),
    b(b"exec", b"a", b"", b"cl"),
    b(b"read", b"adeinNprstu", b"", b"ers"),
    b(b"enable", b"af", b"", b"dnps"),
    b(b"pwd", b"", b"LP", b""),
    b(b"logout", b"", b"", b""),
    b(b"eval", b"", b"", b""),
    b(b"fg", b"", b"", b""),
    b(b"disown", b"h", b"", b"ar"),
    b(b"echo", b"", b"neE", b""),
    b(b".", b"", b"", b""),
    b(b"wait", b"n", b"", b""),
    b(b"jobs", b"x", b"", b"lnprs"),
    b(b"bind", b"mqurfx", b"", b"lpsvPSVX"),
    b(b"caller", b"", b"", b""),
    b(b"times", b"", b"", b""),
    b(b"unalias", b"a", b"", b""),
    b(b"command", b"", b"pVv", b""),
    b(b"trap", b"", b"lp", b""),
    b(b"shift", b"", b"", b""),
    b(b"kill", b"snlL", b"", b""),
    b(b"readarray", b"adeinNprstu", b"", b"ers"),
    b(b"shopt", b"o", b"", b"pqsu"),
    b(b"unset", b"", b"fnv", b""),
];

/// A control keyword's kind.
fn keyword(word: &[u8]) -> Option<u16> {
    Some(match word {
        b"if" => k::IF,
        b"then" => k::THEN,
        b"elif" => k::ELIF,
        b"fi" | b"esac" => k::COND_END,
        b"else" => k::ELSE,
        b"for" => k::FOR,
        b"do" => k::DO,
        b"done" => k::DONE,
        b"while" => k::WHILE,
        b"until" => k::UNTIL,
        b"case" => k::CASE,
        b"continue" => k::CONTINUE_KW,
        b"break" => k::BREAK,
        _ => return None,
    })
}

/// A builtin's index in [`BUILTINS`].
fn builtin(name: &[u8]) -> Option<usize> {
    Some(match name {
        b"set" => 0,
        b"help" => 1,
        b"mapfile" => 2,
        b"getopts" => 3,
        b"cd" => 4,
        b"suspend" => 5,
        b":" => 6,
        b"printf" => 7,
        b"hash" => 8,
        b"ulimit" => 9,
        b"umask" => 10,
        b"source" => 11,
        b"builtin" => 12,
        b"exit" => 13,
        b"type" => 14,
        b"bg" => 15,
        b"return" => 16,
        b"exec" => 17,
        b"read" => 18,
        b"enable" => 19,
        b"pwd" => 20,
        b"logout" => 21,
        b"eval" => 22,
        b"fg" => 23,
        b"disown" => 24,
        b"echo" => 25,
        b"." => 26,
        b"wait" => 27,
        b"jobs" => 28,
        b"bind" => 29,
        b"caller" => 30,
        b"times" => 31,
        b"unalias" => 32,
        b"command" => 33,
        b"trap" => 34,
        b"shift" => 35,
        b"kill" => 36,
        b"readarray" => 37,
        b"shopt" => 38,
        b"unset" => 39,
        _ => return None,
    })
}

/// `\s`, as far as ASCII goes.
fn ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `{{metachar}}`: `[\s|&;()<>]`.
fn metachar(c: u8) -> bool {
    ws(c) || matches!(c, b'|' | b'&' | b';' | b'(' | b')' | b'<' | b'>')
}

/// Byte classes for the runs a context passes whole: a bit per context family, set where the
/// byte starts nothing there.
const ARGS_PLAIN: u8 = 1;
const NAME_PLAIN: u8 = 2;
const WORD_PLAIN: u8 = 4;
const PARAM_PLAIN: u8 = 8;
const EXPR_PLAIN: u8 = 16;
const DEF_PLAIN: u8 = 32;
const FUNC_PLAIN: u8 = 64;
const WORD_GOES_ON: u8 = 128;

static CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        let space = matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
        let meta = space || matches!(b, b'|' | b'&' | b';' | b'(' | b')' | b'<' | b'>');
        // what expansions and strings start on, and the prototype's `\`
        let eas = matches!(
            b,
            b'"' | b'\''
                | b'$'
                | b'?'
                | b'*'
                | b'+'
                | b'@'
                | b'!'
                | b'['
                | b'{'
                | b'`'
                | b'~'
                | b'%'
                | b'\\'
        );
        let mut bits = 0;
        // arguments: redirections start on a digit, `<`, `>`, `&`; `)`, `}` and the separators
        // pop
        if !(meta || eas || b.is_ascii_digit() || b == b'}') {
            bits |= ARGS_PLAIN;
        }
        // a command's name ends on a metacharacter, `` ` ``, `=` or `}`
        if !(meta
            || b == b'`'
            || b == b'='
            || b == b'}'
            || matches!(b, b'"' | b'\'' | b'$' | b'~' | b'%' | b'\\'))
        {
            bits |= NAME_PLAIN;
        }
        // an unquoted value ends on a metacharacter
        if !(meta || eas) {
            bits |= WORD_PLAIN;
        }
        // a parameter's name and its operators
        if b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 {
            bits |= PARAM_PLAIN;
        }
        // arithmetic: names start nothing
        if b.is_ascii_alphabetic() || b == b'_' || b == b'.' || b >= 0x80 {
            bits |= EXPR_PLAIN;
        }
        // an assignment's or an alias's name ends on a metacharacter, `;&` `` ` ``, an
        // assignment's start, and has strings, escapes and `[`
        if !(meta
            || matches!(
                b,
                b';' | b'&' | b'`' | b'=' | b'+' | b'-' | b'?' | b'[' | b'"' | b'\'' | b'$' | b'\\'
            ))
        {
            bits |= DEF_PLAIN;
        }
        // a function's name ends before a space, `(` or `{`
        if !(space || matches!(b, b'(' | b'{' | b'\\')) {
            bits |= FUNC_PLAIN;
        }
        // a statement's first word, as `main`'s lookaheads read it: up to a metacharacter, `=`
        // or a brace
        if !(meta || matches!(b, b'=' | b'{' | b'}')) {
            bits |= WORD_GOES_ON;
        }
        t[c] = bits;
        c += 1;
    }
    t
};

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// syntect's limit: a push at this depth (its stack, ours plus its `__start`) is refused.
const SYNTECT_DEPTH: usize = 99;
/// The bytes the state keeps between lines.
const KEPT: usize = 2 * HALF;

#[derive(Clone, Copy)]
struct Frame {
    ctx: u8,
    /// The embedding whose make patterns come first in it (`makefile::W_*`, 0 for none) in
    /// the low bits; [`IN_OPTION`] and [`IN_PARAM`] above.
    flags: u8,
    /// The kind of the text in it that no pattern takes: its meta content scope's, its meta
    /// scope's, or the one below's.
    base: u16,
    /// A builtin's index, or a here-document's end word's hash ([`tag`]).
    arg: u32,
}

/// [`Frame::flags`]: inside an option ([`TWIN`]).
const IN_OPTION: u8 = 0x40;
/// [`Frame::flags`]: inside a make substitution's `variable.parameter` ([`makefile::TWIN_MK`]).
const IN_PARAM: u8 = 0x80;
/// [`Frame::flags`]: the embedding.
const WITH: u8 = 0x0f;

impl Frame {
    fn with(self) -> u8 {
        self.flags & WITH
    }
}

const FRAME: Frame = Frame {
    ctx: MAIN,
    flags: 0,
    base: PLAIN,
    arg: 0,
};

/// FNV-1a, a here-document's end word.
fn hash(word: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &c in word {
        h = (h ^ u32::from(c)).wrapping_mul(0x0100_0193);
    }
    h
}

/// Lex `b`, a line of bash without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::new(b, state, out, None);
    s.stack[0] = MaybeUninit::new(FRAME);
    s.load(state);
    s.run::<false>();
    s.fill(s.b.len());
    *state = s.save();
}

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
    /// The line's last `]`, once looked for.
    last_bracket: Option<Option<usize>>,
    /// Where the line's last `))` starts, once looked for.
    last_close: Option<Option<usize>>,
    out: &'o mut Out<'p>,
    /// A Makefile's line as its make patterns read it; `None` for a shell script. Behind a
    /// pointer, so that a shell script's scan stays small.
    mk: Option<&'o mut makefile::Mk<'a>>,
    /// Whether the match being done is a make pattern a shell context took first.
    from_with: bool,
    /// The next place an embedding's make patterns may match: `(embedding, from, at)`.
    with_next: (u8, usize, usize),
}

impl<'a, 'o, 'p> Scan<'a, 'o, 'p> {
    #[inline(always)]
    fn new(
        b: &'a [u8],
        state: &State,
        out: &'o mut Out<'p>,
        mk: Option<&'o mut makefile::Mk<'a>>,
    ) -> Self {
        Scan {
            b,
            stack: [const { MaybeUninit::uninit() }; MAX],
            depth: 1,
            emitted: 0,
            push_at: (0, 0),
            guard: false,
            exact: state.is_exact(),
            last_bracket: None,
            last_close: None,
            out,
            mk,
            from_with: false,
            with_next: (0, 0, 0),
        }
    }
}

/// What a pattern does to the stack.
#[derive(Clone, Copy)]
enum Op<'c> {
    None,
    Push(&'c [u8]),
    Set(&'c [u8]),
    Pop,
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

    fn starts(&self, i: usize, s: &[u8]) -> bool {
        self.b.get(i..).is_some_and(|r| r.starts_with(s))
    }

    /// Where the `\s` from `i` ends, the `\n` included.
    fn ws_end(&self, mut i: usize) -> usize {
        while i <= self.b.len() && ws(self.at(i)) {
            i += 1;
        }
        i
    }

    /// Where the run of lowercase ASCII letters from `i` ends.
    fn lower_end(&self, mut i: usize) -> usize {
        while i < self.b.len() && self.b[i].is_ascii_lowercase() {
            i += 1;
        }
        i
    }

    fn digits_end(&self, mut i: usize) -> usize {
        while self.at(i).is_ascii_digit() {
            i += 1;
        }
        i
    }

    /// The character at `i`, past ASCII.
    #[cold]
    fn char_at(&self, i: usize) -> Option<char> {
        let rest = self.b.get(i..)?;
        let len = char_len(rest[0]).min(rest.len());
        std::str::from_utf8(&rest[..len]).ok()?.chars().next()
    }

    #[cold]
    fn char_before(&self, i: usize) -> Option<char> {
        let mut s = i.checked_sub(1)?;
        while s > 0 && self.b[s] & 0xc0 == 0x80 {
            s -= 1;
        }
        std::str::from_utf8(&self.b[s..i]).ok()?.chars().next()
    }

    /// `\w` at `i`.
    fn word_at(&self, i: usize) -> bool {
        match self.at(i) {
            c if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
            _ => self.char_at(i).is_some_and(char::is_alphanumeric),
        }
    }

    /// `\w` before `i`.
    fn word_before(&self, i: usize) -> bool {
        match i.checked_sub(1).map(|j| self.b[j]) {
            None => false,
            Some(c) if c < 0x80 => c.is_ascii_alphanumeric() || c == b'_',
            Some(_) => self.char_before(i).is_some_and(char::is_alphanumeric),
        }
    }

    /// `[[:alpha:]]` at `i`: its length.
    fn alpha_at(&self, i: usize) -> Option<usize> {
        match self.at(i) {
            c if c < 0x80 => c.is_ascii_alphabetic().then_some(1),
            _ => self
                .char_at(i)
                .filter(|c| c.is_alphabetic())
                .map(char::len_utf8),
        }
    }

    /// `{{identifier}}` at `i`: `[[:alpha:]_][[:alnum:]_]*`, where it ends.
    fn identifier(&self, i: usize) -> Option<usize> {
        let first = if self.at(i) == b'_' {
            1
        } else {
            self.alpha_at(i)?
        };
        let mut e = i + first;
        while self.word_at(e) {
            e += char_len(self.at(e));
        }
        Some(e)
    }

    /// `\bWORD{{keyword_break}}` at `p`.
    #[inline]
    fn keyword_at(&self, p: usize, word: &[u8]) -> bool {
        self.at(p) == word[0]
            && self.starts(p, word)
            && !self.word_before(p)
            && self.keyword_break(p + word.len())
    }

    /// `(?![-=\w])` at `i`.
    fn keyword_break(&self, i: usize) -> bool {
        !matches!(self.at(i), b'-' | b'=') && !self.word_at(i)
    }

    /// The place after the character at `p`.
    fn next_char(&self, p: usize) -> usize {
        if p >= self.b.len() {
            p + 1
        } else {
            p + char_len(self.b[p])
        }
    }

    /// Where the bytes of `class` from `p` end, on this line.
    fn run_of(&self, p: usize, class: u8) -> usize {
        let mut q = p;
        while q < self.b.len() && CLASS[usize::from(self.b[q])] & class != 0 {
            q += 1;
        }
        q
    }

    /// Whether a `]` follows `p` on the line: `(?=.*])` after a `[` at `p`.
    fn bracket_after(&mut self, p: usize) -> bool {
        let last = *self
            .last_bracket
            .get_or_insert_with(|| self.b.iter().rposition(|&c| c == b']'));
        last.is_some_and(|i| i > p)
    }

    /// Whether `))` starts at or after `from` on the line.
    fn close_from(&mut self, from: usize) -> bool {
        let last = *self
            .last_close
            .get_or_insert_with(|| self.b.windows(2).rposition(|w| w == b"))"));
        last.is_some_and(|i| i >= from)
    }

    /// The first of `x`, `y`, `z`, `w` from `p` on the line, or the line's end.
    fn find4(&self, p: usize, x: u8, y: u8, z: u8, w: u8) -> usize {
        let rest = &self.b[p.min(self.b.len())..];
        let found = parse(find_byte(eq(x) | eq(y) | eq(z) | eq(w), false), rest);
        match found.result {
            Some(_) => self.b.len() - found.state.len(),
            None => self.b.len(),
        }
    }

    fn find3(&self, p: usize, x: u8, y: u8, z: u8) -> usize {
        let rest = &self.b[p.min(self.b.len())..];
        let found = parse(find_byte(eq(x) | eq(y) | eq(z), false), rest);
        match found.result {
            Some(_) => self.b.len() - found.state.len(),
            None => self.b.len(),
        }
    }

    fn find1(&self, p: usize, x: u8) -> usize {
        let rest = &self.b[p.min(self.b.len())..];
        let found = parse(find_byte(eq(x), false), rest);
        match found.result {
            Some(_) => self.b.len() - found.state.len(),
            None => self.b.len(),
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

    fn frame(&self, ctx: u8, arg: u32) -> Frame {
        let below = *self.top();
        let info = &INFO[usize::from(ctx)];
        let base = if ctx == HEREDOC_LINE {
            // `clear_scopes: 1`: the here-document's string, below it
            self.at_frame(self.depth.saturating_sub(2)).base
        } else if ctx == makefile::EAT_WHITESPACE {
            // `clear_scopes: 1`, where what it clears is all there is
            PLAIN
        } else if info.content != PLAIN {
            info.content
        } else if info.meta != PLAIN {
            info.meta
        } else {
            below.base
        };
        let (keep, set) = FLAGS[usize::from(ctx)];
        let flags = below.flags & keep | set;
        Frame {
            ctx,
            flags,
            base,
            arg,
        }
    }

    fn push_frame(&mut self, ctx: u8, arg: u32) {
        // a context whose one pattern is `""` setting its body, which has the same prototype:
        // its body, at once. The step it took matched nothing, at the place the body then
        // starts from, and changed neither the depth nor the loop guard's place
        let ctx = match ctx {
            CMD_NAME => CMD_NAME_BODY,
            CMD_ARGS => CMD_ARGS_BODY,
            CMD_ARGS_BT => CMD_ARGS_BODY_BT,
            TEST => TEST_BODY,
            TEST_BT => TEST_BODY_BT,
            FOR_ARGS => FOR_BODY,
            BUILTIN_ARGS => BUILTIN_ARGS_BODY,
            BUILTIN_ARGS_BT => BUILTIN_ARGS_BODY_BT,
            // not `HEREDOC_PREAMBLE`: its body, `HEREDOC_LINE`, clears the here-document's
            // string, and the prototype matched before its `""` (`cat <<EOF # c`) keeps it
            // what a recipe's make patterns push sees none of them (syntect's `proto_starts`)
            COMMENT_BODY if self.from_with => COMMENT_BODY_NW,
            CONTINUED if self.from_with => CONTINUED_NW,
            c => c,
        };
        let f = self.frame(ctx, arg);
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

    fn map(&self, kind: u16) -> u16 {
        let flags = self.top().flags;
        if flags & (IN_OPTION | IN_PARAM) == 0 || !is_variable(kind) {
            kind
        } else if flags & IN_PARAM != 0 {
            kind | makefile::TWIN_MK
        } else {
            kind | TWIN
        }
    }

    fn put(&mut self, s: usize, e: usize, kind: u16) {
        let e = e.min(self.b.len());
        if e > s && kind != PLAIN {
            self.out.push_merged(s, e, self.map(kind));
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

    /// Do a match `s..e`: its scope (or [`PLAIN`]), its groups' scopes, its operation, and the
    /// argument the contexts it opens keep. `None` when it is a pop that would loop.
    fn exec(
        &mut self,
        s: usize,
        e: usize,
        scope: u16,
        caps: &[(usize, usize, u16)],
        op: Op,
        arg: u32,
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
                .map(|&c| INFO[usize::from(c)].meta)
                .find(|&m| m != PLAIN)
                .unwrap_or(self.top().base),
            Op::Pop => {
                let ctx = self.top().ctx;
                let meta = INFO[usize::from(ctx)].meta;
                if ctx == makefile::EAT_WHITESPACE {
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
                    self.push_frame(c, arg);
                }
            }
            Op::Set(ctxs) => {
                self.pop_frame();
                for &c in ctxs {
                    self.push_frame(c, arg);
                }
            }
        }
        Some(e)
    }

    fn none(&mut self, s: usize, e: usize, scope: u16) -> Option<usize> {
        self.exec(s, e, scope, &[], Op::None, 0)
    }

    fn push(&mut self, s: usize, e: usize, scope: u16, ctxs: &[u8]) -> Option<usize> {
        self.exec(s, e, scope, &[], Op::Push(ctxs), 0)
    }

    fn set(&mut self, s: usize, e: usize, scope: u16, ctxs: &[u8]) -> Option<usize> {
        self.exec(s, e, scope, &[], Op::Set(ctxs), 0)
    }

    fn pop(&mut self, s: usize, e: usize, scope: u16) -> Option<usize> {
        self.exec(s, e, scope, &[], Op::Pop, 0)
    }

    // ---- the run

    #[inline(always)]
    /// The line, from its start; `MAKE` for a Makefile's, whose make contexts and embedded
    /// shell a shell script's scan never checks for.
    fn run<const MAKE: bool>(&mut self) {
        let mut p = 0;
        loop {
            self.guard = self.push_at == (p, self.depth);
            match self.step::<MAKE>(p) {
                Ok(e) => p = e,
                Err(next) => {
                    if p > self.b.len() || next > self.b.len() + 1 {
                        break;
                    }
                    p = next.max(p + 1).min(self.b.len() + 1);
                }
            }
        }
    }

    /// Match at `p` in the top context: `Ok` with where the match ends, or `Err` with the
    /// next place that may match.
    fn step<const MAKE: bool>(&mut self, p: usize) -> Result<usize, usize> {
        let top = *self.top();
        if !MAKE {
            return self.step_shell(p, top.ctx);
        }
        let with = top.with();
        if with != 0 {
            // a shell context a Makefile embeds: make's patterns first, and no run passed
            // over where one of them may start
            if let Some(e) = self.with_proto(p, with) {
                return Ok(e);
            }
        } else if top.ctx >= MK_FIRST {
            return self.make_step(p, top.ctx);
        }
        let done = self.step_shell(p, top.ctx);
        if with != 0
            && let Err(next) = done
        {
            return Err(next.min(self.next_with(p + 1, with)));
        }
        done
    }

    /// [`Scan::step`] in a shell context.
    fn step_shell(&mut self, p: usize, ctx: u8) -> Result<usize, usize> {
        if INFO[usize::from(ctx)].proto
            && let Some(e) = self.proto(p)
        {
            return Ok(e);
        }
        macro_rules! t {
            ($e:expr) => {
                if let Some(e) = $e {
                    return Ok(e);
                }
            };
        }
        let c = self.at(p);
        match ctx {
            MAIN | SRC_RECIPE | SRC_CALL | SRC_ASSIGN | SRC_EMBED_SQ | SRC_EMBED_DQ
            | SRC_EMBED_BT => {
                t!(self.main(p, false));
            }
            MAIN_POP_AT_END => {
                t!(self.continuation_or_pop(p));
                t!(self.main(p, false));
            }
            COMMENT_BODY | COMMENT_BODY_NW => {
                if p == self.b.len() {
                    t!(self.pop(p, p + 1, k::COMMENT));
                }
                return Err(self.b.len());
            }
            CONTINUED | CONTINUED_NW => {
                if p == 0 {
                    t!(self.pop(0, 0, PLAIN));
                }
                return Err(self.b.len() + 1);
            }
            CMD | CMD_BT => t!(self.cmd(p, ctx == CMD_BT)),
            CMD_PAREN | CMD_PAREN_BT => {
                if c == b')' {
                    let args = if ctx == CMD_PAREN {
                        CMD_ARGS
                    } else {
                        CMD_ARGS_BT
                    };
                    t!(self.set(p, p + 1, k::COMPOUND_END, &[CMD_POST, args]));
                }
                t!(self.main(p, false));
            }
            CMD_BRACE | CMD_BRACE_BT => {
                let bt = ctx == CMD_BRACE_BT;
                if c == b'}' {
                    let args = if bt { CMD_ARGS_BT } else { CMD_ARGS };
                    t!(self.set(p, p + 1, k::BRACES_END, &[CMD_POST, args]));
                }
                t!(self.main(p, bt));
            }
            LET | LET_BT => {
                if p >= self.b.len() || (ctx == LET_BT && c == b'`') {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.expression(p));
            }
            TEST | TEST_BT => {
                let body = if ctx == TEST { TEST_BODY } else { TEST_BODY_BT };
                t!(self.set(p, p, PLAIN, &[body]));
            }
            TEST_BODY | TEST_BODY_BT => {
                t!(self.args_boilerplate(p, ctx == TEST_BODY_BT));
                if ws(c) {
                    let q = self.ws_end(p);
                    if self.at(q) == b']' && q < self.b.len() {
                        t!(self.exec(p, q + 1, PLAIN, &[(q, q + 1, k::TEST_END)], Op::Pop, 0));
                    }
                }
                t!(self.expression_test(p));
            }
            TEST_DOUBLE => {
                if ws(c) {
                    let q = self.ws_end(p);
                    if self.starts(q, b"]]") {
                        t!(self.exec(
                            p,
                            q + 2,
                            PLAIN,
                            &[(q, q + 2, k::DOUBLE_BRACE_END)],
                            Op::Pop,
                            0
                        ));
                    }
                }
                t!(self.expression_test(p));
            }
            CMD_NAME => t!(self.set(p, p, PLAIN, &[CMD_NAME_BODY])),
            CMD_NAME_BODY => {
                // `(?={{is_start_of_arguments}}|{{is_end_of_interpolation}})`
                if p >= self.b.len() || metachar(c) || c == b'`' || c == b'=' {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.name_common(p));
                return Err(self.after(p, NAME_PLAIN));
            }
            CMD_ARGS => t!(self.set(p, p, PLAIN, &[CMD_ARGS_BODY])),
            CMD_ARGS_BT => t!(self.set(p, p, PLAIN, &[CMD_ARGS_BODY_BT])),
            CMD_ARGS_BODY | CMD_ARGS_BODY_BT => {
                let bt = ctx == CMD_ARGS_BODY_BT;
                t!(self.args_boilerplate(p, bt));
                if p == 0 || ws(c) {
                    t!(self.option_start(p, bt));
                }
                return Err(self.after(p, ARGS_PLAIN));
            }
            OPTION | OPTION_BT => {
                if c == b'=' && p < self.b.len() {
                    t!(self.set(p, p, PLAIN, &[OPTION_ASSIGN]));
                }
                // `(?=[^\w$-]|$)`, and a backtick
                if p >= self.b.len() || !(self.word_at(p) || c == b'$' || c == b'-') {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.eas(p));
            }
            OPTION_ASSIGN => {
                if c == b'=' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::OPTION_ASSIGN));
                }
            }
            END_OF_OPTIONS | END_OF_OPTIONS_BT => {
                if ctx == END_OF_OPTIONS_BT && c == b'`' {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.redirection(p));
                if matches!(c, b')' | b'}' | b';' | b'&' | b'|') && p < self.b.len() {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.eas(p));
                t!(self.continuation_or_pop(p));
            }
            CMD_POST => t!(self.cmd_post(p)),
            CASE_WORD => {
                if self.keyword_at(p, b"in") {
                    t!(self.pop(p, p + 2, k::IN));
                }
                if self.keyword_at(p, b"esac") {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.eas(p));
            }
            CASE_BODY => {
                if self.keyword_at(p, b"esac") {
                    t!(self.pop(p, p + 4, k::COND_END));
                }
                if c == b'(' && p < self.b.len() {
                    t!(self.push(p, p, PLAIN, &[CASE_PAREN]));
                }
                if p < self.b.len() && !ws(c) {
                    t!(self.push(p, p, PLAIN, &[CASE_PATTERNS]));
                }
            }
            CASE_PAREN => {
                if c == b'(' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::PATTERNS_BEGIN, &[CASE_PATTERNS]));
                }
            }
            CASE_PATTERNS => {
                if c == b')' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::PATTERNS_END, &[CASE_COMMANDS]));
                }
                if self.case_end(p).is_some() {
                    t!(self.set(p, p, PLAIN, &[CASE_COMMANDS]));
                }
                if self.keyword_at(p, b"esac") {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.case_patterns_body(p));
            }
            CASE_PATTERNS_PAREN => {
                if c == b')' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::PARENS_END));
                }
                t!(self.case_patterns_body(p));
            }
            CASE_COMMANDS => {
                if let Some(e) = self.case_end(p) {
                    t!(self.pop(p, e, k::CASE_END));
                }
                if self.keyword_at(p, b"esac") {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.main(p, false));
            }
            FOR_ARGS => t!(self.set(p, p, PLAIN, &[FOR_BODY])),
            FOR_BODY => {
                t!(self.args_boilerplate(p, false));
                t!(self.arithmetic(p));
                if self.keyword_at(p, b"in") {
                    t!(self.none(p, p + 2, k::IN));
                }
            }
            ARITHMETIC_BODY => {
                if self.starts(p, b"))") {
                    t!(self.pop(p, p + 2, k::ARITHMETIC_END));
                }
                t!(self.expression(p));
                return Err(self.after(p, EXPR_PLAIN));
            }
            EXPRESSION_PAREN => {
                if c == b')' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::PARENS_END));
                }
                t!(self.expression(p));
                return Err(self.after(p, EXPR_PLAIN));
            }
            HEX => {
                if c.is_ascii_alphabetic() && !c.is_ascii_hexdigit() {
                    t!(self.pop(p, p + 1, k::NOT_HEX));
                }
                // `(?=\H)`
                if p < self.b.len() + 1 && !c.is_ascii_hexdigit() {
                    t!(self.pop(p, p, PLAIN));
                }
            }
            OCTAL => {
                if matches!(c, b'8' | b'9') {
                    t!(self.pop(p, p + 1, k::NOT_OCTAL));
                }
                if p < self.b.len() + 1 && !matches!(c, b'0'..=b'7') {
                    t!(self.pop(p, p, PLAIN));
                }
            }
            REGEXP => {
                if ws(c) && p <= self.b.len() {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.eas(p));
            }
            BRACE_BODY => {
                if c == b'}' {
                    t!(self.pop(p, p + 1, k::BRACE_END));
                }
                if c == b',' {
                    t!(self.none(p, p + 1, k::SEPARATOR));
                }
                t!(self.eas(p));
            }
            PARAMETER_START => {
                if c == b'!' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::INDIRECTION, &[PARAMETER_POST]));
                }
                if c == b'#' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::ARITH_OP, &[PARAMETER_POST]));
                }
                t!(self.set(p, p, PLAIN, &[PARAMETER_POST]));
            }
            PARAMETER_POST => {
                t!(self.parameter_post(p));
                return Err(self.after(p, PARAM_PLAIN));
            }
            PARAMETER_OP => {
                let s = p;
                let q = if c == b':' { p + 1 } else { p };
                if matches!(self.at(q), b'-' | b'+' | b'=' | b'?') && q < self.b.len() {
                    t!(self.set(s, q + 1, k::ASSIGN, &[PARAMETER_COMMON]));
                }
            }
            PARAMETER_COMMON => t!(self.parameter_common(p)),
            PARAMETER_PATTERN => {
                t!(self.parameter_common(p));
                t!(self.expansion_pattern(p));
            }
            PARAMETER_SLASH => {
                let q = if matches!(c, b'@' | b'*') { p + 1 } else { p };
                if self.at(q) == b'/' && q < self.b.len() {
                    let mut caps = [(q, q + 1, k::SUBSTITUTION); 2];
                    let n = if q > p {
                        caps[0] = (p, q, k::LANG_VAR);
                        2
                    } else {
                        1
                    };
                    let caps = if n == 2 { &caps[..] } else { &caps[1..] };
                    t!(self.exec(p, q + 1, PLAIN, caps, Op::Set(&[PARAMETER_SLASH_FIRST]), 0));
                }
            }
            PARAMETER_SLASH_FIRST => {
                if matches!(c, b'/' | b'#' | b'%') && p < self.b.len() {
                    t!(self.set(p, p + 1, k::SWITCH, &[PARAMETER_SUBSTITUTE]));
                }
                t!(self.set(p, p, PLAIN, &[PARAMETER_SUBSTITUTE]));
            }
            PARAMETER_SUBSTITUTE => {
                if c == b'/' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::SUBSTITUTION, &[PARAMETER_COMMON]));
                }
                t!(self.parameter_common(p));
                t!(self.expansion_pattern(p));
            }
            PARAMETER_SUBSTRING => {
                let q = if c == b'@' { p + 1 } else { p };
                if self.at(q) == b':' && q < self.b.len() {
                    let caps = [(p, q, k::LANG_VAR), (q, q + 1, k::SUBSTRING_BEGIN)];
                    t!(self.exec(p, q + 1, PLAIN, &caps, Op::Set(&[PARAMETER_OFFSET]), 0));
                }
            }
            PARAMETER_OFFSET => {
                if c == b':' && p < self.b.len() {
                    t!(self.set(p, p, PLAIN, &[PARAMETER_LENGTH_COLON]));
                }
                t!(self.expression(p));
                t!(self.parameter_common(p));
            }
            PARAMETER_LENGTH_COLON => {
                if c == b':' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::SUBSTRING_END, &[PARAMETER_LENGTH]));
                }
            }
            PARAMETER_LENGTH => {
                t!(self.expression(p));
                t!(self.parameter_common(p));
            }
            ARITHMETIC_EXPANSION => {
                if self.starts(p, b"))") {
                    t!(self.pop(p, p + 2, k::PARENS_END));
                }
                t!(self.expression(p));
                return Err(self.after(p, EXPR_PLAIN));
            }
            COMMAND_PARENS => {
                let q = self.ws_end(p);
                if self.at(q) == b')' && q < self.b.len() {
                    t!(self.exec(p, q + 1, PLAIN, &[(q, q + 1, k::PARENS_END)], Op::Pop, 0));
                }
                t!(self.main(p, false));
            }
            BACKTICKS => {
                if c == b'`' {
                    t!(self.pop(p, p + 1, k::GROUP_END));
                }
                t!(self.main(p, true));
            }
            EXTGLOB => {
                if c == b')' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::PARENS_END));
                }
                if c == b'|' {
                    t!(self.none(p, p + 1, k::OR));
                }
                t!(self.eas(p));
            }
            SET_START => {
                if c == b']' {
                    t!(self.set(p, p, PLAIN, &[SET_REST]));
                }
                if matches!(c, b'!' | b'^') {
                    t!(self.set(p, p + 1, k::LOGICAL_NOT, &[SET_REST]));
                }
                if c == b'-' {
                    t!(self.set(p, p + 1, PLAIN, &[SET_REST]));
                }
                t!(self.set(p, p, PLAIN, &[SET_REST]));
            }
            SET_REST => t!(self.set_rest(p)),
            STRING_SINGLE => {
                if c == b'\'' {
                    t!(self.pop(p, p + 1, k::STR_END));
                }
                return Err(self.find1(p + 1, b'\''));
            }
            STRING_DOUBLE | STRING_LOCALE => {
                if c == b'"' {
                    t!(self.pop(p, p + 1, k::STR_END));
                }
                t!(self.dq_escape(p));
                t!(self.expansion_parameter(p));
                t!(self.expansion_arithmetic(p));
                t!(self.expansion_command(p));
                return Err(self.find4(p + 1, b'"', b'\\', b'$', b'`'));
            }
            STRING_ANSI_C => {
                if c == b'\'' {
                    t!(self.pop(p, p + 1, k::STR_END));
                }
                t!(self.dq_escape(p));
                t!(self.ansi_escape(p));
                return Err(self.find3(p + 1, b'\'', b'\\', b'\\'));
            }
            ESCAPED_NEWLINE => {
                if p < self.b.len() && !ws(c) {
                    t!(self.pop(p, p, PLAIN));
                }
            }
            ARRAY_BODY => {
                if c == b']' {
                    t!(self.pop(p, p + 1, k::SECTION_BRACES_END));
                }
                if matches!(c, b'*' | b'@') {
                    t!(self.none(p, p + 1, k::ARRAY_VAR));
                }
                t!(self.expression(p));
            }
            VARDEF_FUNCTION_CALL => t!(self.pop(p, p, PLAIN)),
            VARDEF_MORE => {
                if c == b'`' {
                    t!(self.pop(p, p, PLAIN));
                }
                let q = self.ws_end(p);
                if self.at(q) == b'#' && q < self.b.len() {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.args_boilerplate(p, false));
                if p < self.b.len() && !ws(c) {
                    t!(self.push(p, p, PLAIN, &[VARDEF_VALUE, VARDEF_ASSIGN, VARDEF_NAME]));
                }
            }
            VARDEF_ALIAS_NAME | VARDEF_NAME | FUNCDEF_NAME => {
                let body = match ctx {
                    VARDEF_ALIAS_NAME => VARDEF_ALIAS_NAME_BODY,
                    VARDEF_NAME => VARDEF_NAME_BODY,
                    _ => FUNCDEF_NAME_BODY,
                };
                let q = self.ws_end(p);
                t!(self.set(p, q, PLAIN, &[body]));
            }
            VARDEF_ALIAS_NAME_BODY | VARDEF_NAME_BODY => {
                t!(self.continuation_or_pop(p));
                t!(self.any_escape(p));
                let var = ctx == VARDEF_NAME_BODY;
                if p >= self.b.len()
                    || self.varassign_at(p).is_some()
                    || ws(c)
                    || (var && (metachar(c) || matches!(c, b';' | b'&' | b'`')))
                {
                    t!(self.pop(p, p, PLAIN));
                }
                if c == b'[' {
                    t!(self.push(p, p + 1, k::SECTION_BRACES_BEGIN, &[ARRAY_BODY]));
                }
                // `\s*$`, the line's `\n` included
                if p < self.b.len() && self.b[p..].iter().all(|&c| ws(c)) {
                    t!(self.pop(p, self.b.len() + 1, PLAIN));
                }
                t!(self.string(p));
                return Err(self.run_of(self.next_char(p), DEF_PLAIN));
            }
            VARDEF_ASSIGN => {
                t!(self.continuation_or_pop(p));
                t!(self.any_escape(p));
                if let Some(e) = self.varassign_at(p) {
                    t!(self.pop(p, e, k::ASSIGN));
                }
                t!(self.pop(p, p, PLAIN));
            }
            VARDEF_VALUE => {
                if c == b'(' && p < self.b.len() {
                    t!(self.set(p, p + 1, k::PARENS_BEGIN, &[VARDEF_ARRAY]));
                }
                if matches!(c, b'&' | b'`') {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.set(p, p, PLAIN, &[VARDEF_VALUE_WORD]));
            }
            VARDEF_ARRAY => {
                if c == b')' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::PARENS_END));
                }
                if c == b'[' {
                    t!(self.push(p, p + 1, k::BRACKETS_BEGIN, &[VARDEF_ARRAY_INDEX]));
                }
                t!(self.eas(p));
            }
            VARDEF_ARRAY_INDEX => {
                if c == b']' {
                    t!(self.set(p, p + 1, k::BRACKETS_END, &[VARDEF_ARRAY_EQ]));
                }
                t!(self.eas(p));
            }
            VARDEF_ARRAY_EQ => {
                if c == b'=' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::ASSIGN));
                }
                t!(self.pop(p, p, PLAIN));
            }
            VARDEF_VALUE_WORD => {
                if c == b'`' {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.eas(p));
                t!(self.continuation_or_pop(p));
                t!(self.any_escape(p));
                if metachar(c) && p <= self.b.len() {
                    t!(self.pop(p, p, PLAIN));
                }
                return Err(self.after(p, WORD_PLAIN));
            }
            VARDEF_READONLY_OPTIONS
            | VARDEF_EXPORT_OPTIONS
            | VARDEF_DECLARE_OPTIONS
            | VARDEF_ALIAS_OPTIONS => {
                let set: &[u8] = match ctx {
                    VARDEF_READONLY_OPTIONS => b"aAf",
                    VARDEF_EXPORT_OPTIONS => b"fn",
                    VARDEF_DECLARE_OPTIONS => b"aAfFgilnrtux",
                    _ => b"",
                };
                let q = self.ws_end(p);
                if self.at(q) == b'-' && q < self.b.len() {
                    let mut e = q + 1;
                    while set.contains(&self.at(e)) && e < self.b.len() {
                        e += 1;
                    }
                    if e == q + 1 && self.at(e) == b'p' && e < self.b.len() {
                        e += 1;
                    }
                    if e > q + 1 {
                        let caps = [(q, e, k::OPTION), (q, q + 1, k::OPTION_MARK)];
                        t!(self.exec(p, e, PLAIN, &caps, Op::None, 0));
                    }
                }
                t!(self.pop(p, q, PLAIN));
            }
            PROCESS_SUBSTITUTION => {
                if c == b')' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::PARENS_END));
                }
                t!(self.main(p, false));
            }
            REDIRECTION_POST => {
                let q = self.ws_end(p);
                let d = self.at(q);
                if d.is_ascii_digit() && q < self.b.len() {
                    let e = self.digits_end(q);
                    t!(self.exec(p, e, PLAIN, &[(q, e, k::FD)], Op::Pop, 0));
                }
                if d == b'-' && q < self.b.len() {
                    t!(self.exec(p, q + 1, PLAIN, &[(q, q + 1, k::FD_END)], Op::Pop, 0));
                }
                if q < self.b.len() {
                    t!(self.set(p, q, PLAIN, &[REDIRECTION_TARGET]));
                }
                t!(self.pop(p, q, PLAIN));
            }
            REDIRECTION_TARGET => {
                if p <= self.b.len() && (metachar(c) || c == b'`') {
                    t!(self.pop(p, p, PLAIN));
                }
                t!(self.eas(p));
            }
            HEREDOC | HEREDOC_TABS | HEREDOC_LITERAL | HEREDOC_TABS_LITERAL => {
                let expand = matches!(ctx, HEREDOC | HEREDOC_TABS);
                if expand {
                    t!(self.heredoc_escape(p));
                    t!(self.expansion_parameter(p));
                    t!(self.expansion_arithmetic(p));
                    t!(self.expansion_command(p));
                }
                if p == 0 {
                    t!(self.heredoc_end(matches!(ctx, HEREDOC_TABS | HEREDOC_TABS_LITERAL)));
                }
                if expand {
                    return Err(self.find3(p + 1, b'\\', b'$', b'`'));
                }
                return Err(self.b.len() + 1);
            }
            HEREDOC_PREAMBLE => t!(self.set(p, p, PLAIN, &[HEREDOC_LINE])),
            HEREDOC_LINE => {
                if p >= self.b.len() {
                    t!(self.pop(p, p, PLAIN));
                }
                let q = self.ws_end(p);
                if q < self.b.len() {
                    t!(self.push(p, q, PLAIN, &[MAIN_POP_AT_END, CMD_POST, CMD_ARGS]));
                }
            }
            COPROC_BODY => {
                let q = self.ws_end(p);
                if q < self.b.len() {
                    let mut k = q;
                    while k < self.b.len() && !ws(self.b[k]) {
                        k += 1;
                    }
                    let brace = self.b[q + 1..k].contains(&b'{')
                        || (self.at(self.ws_end(k)) == b'{' && self.ws_end(k) < self.b.len());
                    if brace {
                        t!(self.set(p, q, PLAIN, &[COPROC_NAME_CTX]));
                    }
                }
                t!(self.set(p, p, PLAIN, &[MAIN_POP_AT_END]));
            }
            COPROC_NAME_CTX => {
                let q = self.ws_end(p);
                if self.at(q) == b'{' && q < self.b.len() {
                    t!(self.set(p, p, PLAIN, &[COPROC_BRACE]));
                }
            }
            COPROC_BRACE => {
                if c == b'{' {
                    t!(self.set(p, p + 1, k::SECTION_BRACES_BEGIN, &[COPROC_BRACES]));
                }
            }
            COPROC_BRACES => {
                if c == b'}' {
                    t!(self.pop(p, p + 1, k::SECTION_BRACES_END));
                }
                t!(self.main(p, false));
            }
            FUNCDEF_NAME_BODY => {
                // `(?=\s*[({]|$)`
                let q = self.ws_end(p);
                if p >= self.b.len() || (matches!(self.at(q), b'(' | b'{') && q < self.b.len()) {
                    t!(self.pop(p, p, PLAIN));
                }
                return Err(self.run_of(self.next_char(p), FUNC_PLAIN));
            }
            FUNCDEF_PARENS => {
                if c == b'(' && p < self.b.len() {
                    let q = self.ws_end(p + 1);
                    if self.at(q) == b')' && q < self.b.len() {
                        let caps = [(p, p + 1, k::PARENS_BEGIN), (q, q + 1, k::PARENS_END)];
                        t!(self.exec(p, q + 1, PLAIN, &caps, Op::None, 0));
                    }
                }
                if c == b'{' {
                    t!(self.pop(p, p + 1, k::SECTION_BRACES_BEGIN));
                }
                if c == b'(' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::COMPOUND_BEGIN));
                }
            }
            FUNCDEF_BODY | FUNCDEF_BODY_BT => {
                if c == b'}' {
                    t!(self.pop(p, p + 1, k::SECTION_BRACES_END));
                }
                if c == b')' && p < self.b.len() {
                    t!(self.pop(p, p + 1, k::COMPOUND_END));
                }
                t!(self.main(p, ctx == FUNCDEF_BODY_BT));
            }
            BUILTIN_ARGS | BUILTIN_ARGS_BT => {
                let body = if ctx == BUILTIN_ARGS {
                    BUILTIN_ARGS_BODY
                } else {
                    BUILTIN_ARGS_BODY_BT
                };
                let arg = self.top().arg;
                t!(self.exec(p, p, PLAIN, &[], Op::Set(&[body]), arg));
            }
            BUILTIN_ARGS_BODY | BUILTIN_ARGS_BODY_BT => {
                t!(self.args_boilerplate(p, ctx == BUILTIN_ARGS_BODY_BT));
                t!(self.builtin_option(p));
                return Err(self.after(p, ARGS_PLAIN));
            }
            BUILTIN_END_OF_OPTIONS => {
                t!(self.eas(p));
                t!(self.continuation_or_pop(p));
            }
            _ => {}
        }
        // at the line's end, `main` and the closing bytes of the contexts around it match
        // nothing
        if p >= self.b.len() && ends_with_main(ctx) {
            return Err(usize::MAX);
        }
        if p < self.b.len() && ws(c) && skips_spaces(ctx) {
            return Err(self.ws_end(p).min(self.b.len()));
        }
        Err(self.next_char(p))
    }

    /// The next place that may match after `p`, where nothing did: past a run of spaces, or
    /// past bytes of `class`.
    fn after(&self, p: usize, class: u8) -> usize {
        if p < self.b.len() && ws(self.b[p]) {
            self.ws_end(p).min(self.b.len())
        } else {
            self.run_of(self.next_char(p), class)
        }
    }

    // ---- the prototype and the line's end

    /// `comment`, `line-continuation`, `any-escape`.
    #[inline(always)]
    fn proto(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        // `(?:^\s*|\s+)(\#)`
        if p == 0 || ws(c) {
            let q = self.ws_end(p);
            if self.at(q) == b'#' && q < self.b.len() {
                let caps = [(q, q + 1, k::COMMENT_BEGIN)];
                return self.exec(p, q + 1, PLAIN, &caps, Op::Push(&[COMMENT_BODY]), 0);
            }
        }
        if c == b'\\' && p < self.b.len() {
            return self.backslash(p, true);
        }
        None
    }

    /// At a `\`: `\\\n` (pushing the context that ends at the next line's start), `\\(\s+)\n`,
    /// and, when `escape`, `\\.`.
    fn backslash(&mut self, p: usize, escape: bool) -> Option<usize> {
        if p + 1 == self.b.len() {
            return self.push(p, self.b.len() + 1, k::CONTINUATION, &[CONTINUED]);
        }
        if self.b[p + 1..].iter().all(|&c| ws(c)) {
            let caps = [(p + 1, self.b.len(), k::EXTRA_SPACES)];
            return self.exec(p, self.b.len() + 1, PLAIN, &caps, Op::None, 0);
        }
        if escape {
            return self.none(p, p + 1 + char_len(self.b[p + 1]), k::ESCAPE);
        }
        None
    }

    /// `\\.`
    fn any_escape(&mut self, p: usize) -> Option<usize> {
        if self.at(p) == b'\\' && p + 1 < self.b.len() {
            return self.none(p, p + 1 + char_len(self.b[p + 1]), k::ESCAPE);
        }
        None
    }

    /// `line-continuation-or-pop-at-end`: `$`, `\\\n`, `\\(\s+)\n`.
    fn continuation_or_pop(&mut self, p: usize) -> Option<usize> {
        if p >= self.b.len() {
            return self.pop(p, p, PLAIN);
        }
        if self.b[p] == b'\\' {
            return self.backslash(p, false);
        }
        None
    }

    // ---- statements

    /// `main` (`main-bt` when `bt`).
    fn main(&mut self, p: usize, bt: bool) -> Option<usize> {
        let q = self.ws_end(p);
        let c = self.at(q);
        // where the first word stops (a metacharacter, `=` or a brace), read once for both
        // lookaheads below
        let stop = self.run_of(q, WORD_GOES_ON);
        if q < self.b.len() {
            // funcdef: `\s*\b(function)\s+`, `(?=\s*{{identifier_non_posix}}\s*\(\s*\))`
            let body = if bt { FUNCDEF_BODY_BT } else { FUNCDEF_BODY };
            let funcdef = [body, FUNCDEF_PARENS, FUNCDEF_NAME];
            if c == b'f' && self.starts(q, b"function") && !self.word_before(q) {
                let e = self.ws_end(q + 8);
                if e > q + 8 {
                    let caps = [(q, q + 8, k::FUNCTION_KW)];
                    return self.exec(p, e, PLAIN, &caps, Op::Push(&funcdef), 0);
                }
            }
            if self.function_name_from(q, stop) {
                return self.push(p, p, PLAIN, &funcdef);
            }
        }
        if self.keyword_at(p, b"coproc") {
            let args = if bt { CMD_ARGS_BT } else { CMD_ARGS };
            return self.push(p, p + 6, k::COPROC, &[CMD_POST, args, COPROC_BODY]);
        }
        if q < self.b.len() {
            // vardef: `\s*\b(WORD){{keyword_break}}`, the word read once
            let e = self.lower_end(q);
            let modifier = match &self.b[q..e] {
                b"alias" => Some((k::ALIAS, VARDEF_ALIAS_NAME, VARDEF_ALIAS_OPTIONS)),
                b"typeset" | b"declare" | b"local" => {
                    Some((k::MODIFIER, VARDEF_NAME, VARDEF_DECLARE_OPTIONS))
                }
                b"export" => Some((k::MODIFIER, VARDEF_NAME, VARDEF_EXPORT_OPTIONS)),
                b"readonly" => Some((k::MODIFIER, VARDEF_NAME, VARDEF_READONLY_OPTIONS)),
                _ => None,
            };
            if let Some((kind, name, options)) = modifier
                && !self.word_before(q)
                && self.keyword_break(e)
            {
                let ctxs = [
                    VARDEF_FUNCTION_CALL,
                    VARDEF_MORE,
                    VARDEF_VALUE,
                    VARDEF_ASSIGN,
                    name,
                    options,
                ];
                return self.exec(p, e, PLAIN, &[(q, e, kind)], Op::Push(&ctxs), 0);
            }
            if self.is_variable_from(q, stop) {
                return self.push(p, p, PLAIN, &[VARDEF_VALUE, VARDEF_ASSIGN, VARDEF_NAME]);
            }
        }
        if let Some(e) = self.redirection(p) {
            return Some(e);
        }
        let c = self.at(p);
        if !bt && c == b'!' && p < self.b.len() {
            // `\!(?!\S)`, `(\!)(-?\d+|!)`, `\!`
            let d = self.at(p + 1);
            if p + 1 >= self.b.len() || ws(d) {
                return self.none(p, p + 1, k::LOGICAL);
            }
            if d == b'!' {
                return self.exec(
                    p,
                    p + 2,
                    k::HISTORY,
                    &[(p, p + 1, k::HISTORY_MARK)],
                    Op::None,
                    0,
                );
            }
            let s = if d == b'-' { p + 2 } else { p + 1 };
            let e = self.digits_end(s);
            if e > s {
                return self.exec(
                    p,
                    e,
                    k::HISTORY,
                    &[(p, p + 1, k::HISTORY_MARK)],
                    Op::None,
                    0,
                );
            }
            return self.none(p, p + 1, k::HISTORY_MARK);
        }
        if p < self.b.len() && !ws(c) {
            return self.push(p, p, PLAIN, &[if bt { CMD_BT } else { CMD }]);
        }
        None
    }

    /// [`Scan::function_name`], given where the first word from `q` stops: decided there
    /// when it stops at a metacharacter or the line's end, where the name's run ends too.
    fn function_name_from(&self, q: usize, stop: usize) -> bool {
        if stop < self.b.len() && !metachar(self.b[stop]) {
            return self.function_name(q);
        }
        let c = self.at(q);
        if metachar(c) || c.is_ascii_digit() {
            return false;
        }
        let open = self.ws_end(stop.max(q + 1));
        self.at(open) == b'(' && open < self.b.len() && {
            let close = self.ws_end(open + 1);
            self.at(close) == b')' && close < self.b.len()
        }
    }

    /// [`Scan::is_variable`], given where the first word from `q` stops: decided there when
    /// it stops at `=`, a space or the line's end, where the run of `{{nbc}}` ends too.
    fn is_variable_from(&self, q: usize, stop: usize) -> bool {
        if stop >= self.b.len() {
            return false;
        }
        match self.b[stop] {
            b'=' => true,
            c if ws(c) => false,
            _ => self.is_variable(q),
        }
    }

    /// `(?=\s*{{identifier_non_posix}}\s*\(\s*\))` from `q`, past the spaces.
    fn function_name(&self, q: usize) -> bool {
        let c = self.at(q);
        if metachar(c) || c.is_ascii_digit() {
            return false;
        }
        let mut e = q + 1;
        while e < self.b.len() && !metachar(self.b[e]) && self.b[e] != b'=' {
            e += 1;
        }
        let open = self.ws_end(e);
        self.at(open) == b'(' && open < self.b.len() && {
            let close = self.ws_end(open + 1);
            self.at(close) == b')' && close < self.b.len()
        }
    }

    /// `{{is_variable}}` from `q`, past the spaces: `{{nbc}}(?:[({]{{nbc}}[)}])?{{nbc}}=`.
    fn is_variable(&self, q: usize) -> bool {
        let nbc = |c: u8| !(matches!(c, b'{' | b'}' | b'(' | b')' | b'=') || ws(c));
        let run = |mut i: usize| {
            while i < self.b.len() && nbc(self.b[i]) {
                i += 1;
            }
            i
        };
        let mut i = run(q);
        if matches!(self.at(i), b'(' | b'{') && i < self.b.len() {
            let j = run(i + 1);
            if matches!(self.at(j), b')' | b'}') && j < self.b.len() {
                i = run(j + 1);
            }
        }
        self.at(i) == b'=' && i < self.b.len()
    }

    /// `cmd` (`cmd-bt`), at its first byte.
    fn cmd(&mut self, p: usize, bt: bool) -> Option<usize> {
        let c = self.at(p);
        // the word at `p`, read once: a keyword or a builtin is the whole of it (each needs a
        // break after it that a lowercase letter isn't), so one `match` stands for the
        // grammar's patterns in order
        let e = self.lower_end(p);
        let word = self.b.get(p..e).unwrap_or_default();
        // control
        if !word.is_empty()
            && !self.word_before(p)
            && self.keyword_break(e)
            && let Some(kind) = keyword(word)
        {
            return match word {
                b"if" | b"then" | b"elif" | b"else" | b"do" | b"esac" => self.pop(p, e, kind),
                b"fi" | b"done" | b"break" => self.set(p, e, kind, &[CMD_POST, CMD_ARGS]),
                b"for" => self.set(p, e, kind, &[CMD_POST, FOR_ARGS]),
                b"case" => self.set(p, e, kind, &[CASE_BODY, CASE_WORD]),
                _ => self.none(p, e, kind),
            };
        }
        // arithmetic
        if let Some(e) = self.arithmetic(p) {
            return Some(e);
        }
        if matches!(c, b')' | b'}')
            && p < self.b.len()
            && let Some(e) = self.pop(p, p, PLAIN)
        {
            return Some(e);
        }
        if let Some(e) = self.continuation_or_pop(p) {
            return Some(e);
        }
        if c == b'(' && p < self.b.len() {
            let paren = if bt { CMD_PAREN_BT } else { CMD_PAREN };
            return self.push(p, p + 1, k::COMPOUND_BEGIN, &[paren]);
        }
        // the builtins: their names anywhere, `{{boundary_end}}` after
        let name_end = if matches!(c, b'.' | b':') { p + 1 } else { e };
        if let Some(i) = builtin(self.b.get(p..name_end).unwrap_or_default()) {
            let d = self.at(name_end);
            if name_end >= self.b.len() || ws(d) || matches!(d, b';' | b'>' | b'<' | b'|') {
                let args = if bt { BUILTIN_ARGS_BT } else { BUILTIN_ARGS };
                let kind = k::B_SET + i as u16;
                return self.exec(p, name_end, kind, &[], Op::Set(&[CMD_POST, args]), i as u32);
            }
        }
        if word == b"let" && !self.word_before(p) && !self.word_at(e) {
            return self.push(p, p + 3, k::LET, &[if bt { LET_BT } else { LET }]);
        }
        if self.starts(p, b"[[") && ws(self.at(p + 2)) {
            let caps = [(p, p + 2, k::DOUBLE_BRACE_BEGIN)];
            return self.exec(p, p + 2, PLAIN, &caps, Op::Set(&[CMD_POST, TEST_DOUBLE]), 0);
        }
        if c == b'[' && ws(self.at(p + 1)) {
            let test = if bt { TEST_BT } else { TEST };
            let caps = [(p, p + 1, k::TEST_BEGIN)];
            return self.exec(p, p + 1, PLAIN, &caps, Op::Set(&[CMD_POST, test]), 0);
        }
        if c == b'{' && ws(self.at(p + 1)) {
            let brace = if bt { CMD_BRACE_BT } else { CMD_BRACE };
            let caps = [(p, p + 1, k::BRACES_BEGIN)];
            return self.exec(p, p + 1, PLAIN, &caps, Op::Push(&[brace]), 0);
        }
        if p < self.b.len() && !ws(c) {
            let args = if bt { CMD_ARGS_BT } else { CMD_ARGS };
            return self.set(p, p, PLAIN, &[CMD_POST, args, CMD_NAME]);
        }
        None
    }

    /// `arithmetic`: `\(\((?=.+\)\))`
    fn arithmetic(&mut self, p: usize) -> Option<usize> {
        if self.starts(p, b"((") && self.close_from(p + 3) {
            return self.push(p, p + 2, k::ARITHMETIC_BEGIN, &[ARITHMETIC_BODY]);
        }
        None
    }

    fn cmd_post(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        let d = self.at(p + 1);
        match c {
            b';' if !matches!(d, b';' | b'&') => self.pop(p, p + 1, k::CONTINUE),
            b'|' if d == b'|' => self.pop(p, p + 2, k::OR),
            b'|' => self.pop(p, p + 1, k::PIPE),
            b'&' if d == b'&' => self.pop(p, p + 2, k::AND),
            b'&' => self.pop(p, p + 1, k::JOB),
            _ if p >= self.b.len() || !ws(c) => self.pop(p, p, PLAIN),
            _ => None,
        }
    }

    /// `cmd-name-common`, after the name's own end.
    fn name_common(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        // `(?=}|\s+#|\s*(?:[|;]|&(?!>)))`: a space or a separator has ended the name already
        if c == b'}' {
            return self.pop(p, p, PLAIN);
        }
        match c {
            b'"' | b'\'' => self.string(p),
            b'$' => {
                if let Some(e) = self.string(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_parameter(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_arithmetic(p) {
                    return Some(e);
                }
                self.expansion_command(p)
            }
            b'`' => self.expansion_command(p),
            b'~' => self.none(p, p + 1, k::TILDE),
            b'%' => self.expansion_job(p),
            b'\\' => self.continuation_or_pop(p),
            _ => None,
        }
    }

    /// `cmd-args-boilerplate` (`-bt`): `(?=\))`, `cmd-args-common`, `--`.
    fn args_boilerplate(&mut self, p: usize, bt: bool) -> Option<usize> {
        // by the byte at `p`, the patterns that can start on it, in the grammar's order:
        // `(?=\))`, `(?=}|\s+#)`, `redirection`, `(?=\s*([|;]|&(?!>)))`,
        // `expansion-and-string`, `line-continuation-or-pop-at-end`, `(?:\s+|^)--(?=\s|$)`
        if p >= self.b.len() {
            return self.continuation_or_pop(p);
        }
        let c = self.b[p];
        let eoo = if bt {
            END_OF_OPTIONS_BT
        } else {
            END_OF_OPTIONS
        };
        if ws(c) {
            let q = self.ws_end(p);
            if q >= self.b.len() {
                return None;
            }
            let d = self.b[q];
            if d == b'#'
                && let Some(e) = self.pop(p, p, PLAIN)
            {
                return Some(e);
            }
            if (matches!(d, b'|' | b';') || (d == b'&' && self.at(q + 1) != b'>'))
                && let Some(e) = self.pop(p, p, PLAIN)
            {
                return Some(e);
            }
            if d == b'-' && self.starts(q, b"--") && ws(self.at(q + 2)) {
                return self.set(p, q + 2, k::END_OF_OPTIONS, &[eoo]);
            }
            return None;
        }
        match c {
            b')' | b'}' => self.pop(p, p, PLAIN),
            b'`' => {
                if bt && let Some(e) = self.pop(p, p, PLAIN) {
                    return Some(e);
                }
                self.eas(p)
            }
            b'0'..=b'9' | b'<' | b'>' => self.redirection(p),
            b'&' => {
                if let Some(e) = self.redirection(p) {
                    return Some(e);
                }
                if self.at(p + 1) != b'>' {
                    return self.pop(p, p, PLAIN);
                }
                None
            }
            b'|' | b';' => self.pop(p, p, PLAIN),
            b'\\' => self.continuation_or_pop(p),
            b'-' if p == 0 && self.starts(0, b"--") && ws(self.at(2)) => {
                self.set(0, 2, k::END_OF_OPTIONS, &[eoo])
            }
            _ => self.eas(p),
        }
    }

    /// `(?:\s+|^)` at `p`: where what follows it starts.
    fn ws_or_bol(&self, p: usize) -> Option<usize> {
        if ws(self.at(p)) && p < self.b.len() {
            Some(self.ws_end(p))
        } else if p == 0 {
            Some(0)
        } else {
            None
        }
    }

    /// `{{start_of_option}}`: `(?:\s+|^)--?(?=[\w$])`
    fn option_start(&mut self, p: usize, bt: bool) -> Option<usize> {
        let k = self.ws_or_bol(p)?;
        if self.at(k) != b'-' || k >= self.b.len() {
            return None;
        }
        let opens = |s: &Self, i: usize| s.word_at(i) || s.at(i) == b'$';
        let e = if self.at(k + 1) == b'-' && opens(self, k + 2) {
            k + 2
        } else if opens(self, k + 1) {
            k + 1
        } else {
            return None;
        };
        let option = if bt { OPTION_BT } else { OPTION };
        self.push(p, e, k::OPTION_MARK, &[option])
    }

    /// A builtin's options, `(?:\s+|^)((PREFIX)(?:[ONE]|[MANY]+)|[BARE]+)(?=\s|;|$|\`|\))`.
    fn builtin_option(&mut self, p: usize) -> Option<usize> {
        let bi = BUILTINS.get(self.top().arg as usize)?;
        if bi.one.is_empty() && bi.many.is_empty() && bi.bare.is_empty() {
            return None;
        }
        let k = self.ws_or_bol(p)?;
        if k >= self.b.len() {
            return None;
        }
        let ends = |s: &Self, i: usize| {
            i >= s.b.len() || ws(s.at(i)) || matches!(s.at(i), b';' | b'`' | b')')
        };
        let run = |s: &Self, from: usize, set: &[u8]| {
            let mut e = from;
            while e < s.b.len() && set.contains(&s.b[e]) {
                e += 1;
            }
            e
        };
        if bi.prefix.contains(&self.b[k]) {
            let j = k + 1;
            if j < self.b.len() && bi.one.contains(&self.b[j]) && ends(self, j + 1) {
                let caps = [(k, j + 1, k::OPTION), (k, j, k::OPTION_MARK)];
                return self.exec(p, j + 1, PLAIN, &caps, Op::None, 0);
            }
            let e = run(self, j, bi.many);
            if e > j && ends(self, e) {
                let caps = [(k, e, k::OPTION), (k, j, k::OPTION_MARK)];
                return self.exec(p, e, PLAIN, &caps, Op::None, 0);
            }
        }
        let e = run(self, k, bi.bare);
        if e > k && ends(self, e) {
            return self.exec(p, e, PLAIN, &[(k, e, k::OPTION)], Op::None, 0);
        }
        None
    }

    /// `;;&?|;&` at `p`: where it ends.
    fn case_end(&self, p: usize) -> Option<usize> {
        if self.starts(p, b";;&") {
            Some(p + 3)
        } else if self.starts(p, b";;") || self.starts(p, b";&") {
            Some(p + 2)
        } else {
            None
        }
    }

    /// `case-clause-patterns-body`.
    fn case_patterns_body(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.expansion_pattern(p) {
            return Some(e);
        }
        let c = self.at(p);
        match c {
            b'~' => return self.none(p, p + 1, k::TILDE),
            b'$' => {
                if let Some(e) = self.expansion_parameter(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_command(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_arithmetic(p) {
                    return Some(e);
                }
            }
            b'`' => return self.expansion_command(p),
            _ => {}
        }
        if let Some(e) = self.string(p) {
            return Some(e);
        }
        match c {
            b'|' => self.none(p, p + 1, k::LOGICAL),
            b'(' if p < self.b.len() => {
                self.push(p, p + 1, k::PARENS_BEGIN, &[CASE_PATTERNS_PAREN])
            }
            _ => None,
        }
    }

    // ---- redirections

    /// `redirection`, at `(\d*)`.
    fn redirection(&mut self, p: usize) -> Option<usize> {
        let d = self.digits_end(p);
        let c = self.at(d);
        if !matches!(c, b'<' | b'>' | b'&') || d >= self.b.len() {
            return None;
        }
        let fd = (p, d, k::FD);
        if c == b'<' {
            // `(<<<)\s`
            if self.starts(d, b"<<<") && ws(self.at(d + 3)) {
                let caps = [fd, (d, d + 3, k::HERE_STRING)];
                return self.exec(p, d + 4, PLAIN, &caps, Op::None, 0);
            }
            if self.starts(d, b"<<")
                && let Some(e) = self.heredoc(p, d)
            {
                return Some(e);
            }
        }
        // `([<>])(\()`
        if matches!(c, b'<' | b'>') && self.at(d + 1) == b'(' {
            let caps = [
                fd,
                (d, d + 1, k::REDIRECT_PROCESS),
                (d + 1, d + 2, k::PARENS_BEGIN),
            ];
            return self.exec(p, d + 2, PLAIN, &caps, Op::Push(&[PROCESS_SUBSTITUTION]), 0);
        }
        let e = match c {
            // `(<&?)`
            b'<' => {
                if self.at(d + 1) == b'&' {
                    d + 2
                } else {
                    d + 1
                }
            }
            // `(>>!?|>&?|&>|&?>(?:\||>))`
            b'>' if self.at(d + 1) == b'>' => {
                if self.at(d + 2) == b'!' {
                    d + 3
                } else {
                    d + 2
                }
            }
            b'>' if self.at(d + 1) == b'&' => d + 2,
            b'>' => d + 1,
            _ if self.at(d + 1) == b'>' => d + 2,
            _ => return None,
        };
        let caps = [fd, (d, e, k::REDIRECT)];
        self.exec(p, e, PLAIN, &caps, Op::Push(&[REDIRECTION_POST]), 0)
    }

    /// The eight here-document forms, at `<<`.
    fn heredoc(&mut self, p: usize, d: usize) -> Option<usize> {
        let fd = (p, d, k::FD);
        for dash in [true, false] {
            let o = if dash {
                if !self.starts(d, b"<<-") {
                    continue;
                }
                d + 3
            } else {
                d + 2
            };
            let literal = if dash {
                HEREDOC_TABS_LITERAL
            } else {
                HEREDOC_LITERAL
            };
            let bare = if dash { HEREDOC_TABS } else { HEREDOC };
            let j = self.ws_end(o);
            let op = (d, o, k::REDIRECT);
            // `'…'` and `"…"`
            for quote in *b"'\"" {
                if self.at(j) == quote
                    && j < self.b.len()
                    && let Some(e) = self.identifier(j + 1)
                    && self.at(e) == quote
                    && e < self.b.len()
                {
                    let caps = [
                        fd,
                        op,
                        (j, j + 1, k::STR_BEGIN),
                        (j + 1, e, k::HEREDOC_TOKEN),
                        (e, e + 1, k::STR_END),
                    ];
                    let arg = tag(&self.b[j + 1..e]);
                    return self.exec(
                        p,
                        e + 1,
                        PLAIN,
                        &caps,
                        Op::Push(&[literal, HEREDOC_PREAMBLE]),
                        arg,
                    );
                }
            }
            // `\WORD`
            if self.at(j) == b'\\'
                && j < self.b.len()
                && let Some(e) = self.identifier(j + 1)
            {
                let caps = [
                    fd,
                    op,
                    (j, j + 1, k::STRING_MARK),
                    (j + 1, e, k::HEREDOC_TOKEN),
                ];
                let arg = tag(&self.b[j + 1..e]);
                return self.exec(
                    p,
                    e,
                    PLAIN,
                    &caps,
                    Op::Push(&[literal, HEREDOC_PREAMBLE]),
                    arg,
                );
            }
            if j < self.b.len()
                && let Some(e) = self.identifier(j)
            {
                let caps = [fd, op, (j, e, k::HEREDOC_TOKEN)];
                let arg = tag(&self.b[j..e]);
                return self.exec(p, e, PLAIN, &caps, Op::Push(&[bare, HEREDOC_PREAMBLE]), arg);
            }
        }
        None
    }

    /// A here-document's end, at the line's start: `^\3(\s+)\n` and `^\3$`, after spaces too
    /// when `tabs`.
    fn heredoc_end(&mut self, tabs: bool) -> Option<usize> {
        let arg = self.top().arg;
        let i = if tabs {
            self.ws_end(0).min(self.b.len())
        } else {
            0
        };
        // the identifier the line starts with: the word, if the line ends the here-document
        let e = if i < self.b.len() {
            self.identifier(i)?
        } else {
            return None;
        };
        if hash(&self.b[i..e]) != arg {
            return None;
        }
        if e < self.b.len() && self.b[e..].iter().all(|&c| ws(c)) {
            return self.exec(
                0,
                self.b.len() + 1,
                PLAIN,
                &[(e, self.b.len(), k::HEREDOC_SPACES)],
                Op::Pop,
                0,
            );
        }
        if e == self.b.len() {
            return self.exec(0, e, PLAIN, &[(i, e, k::HEREDOC_TOKEN)], Op::Pop, 0);
        }
        None
    }

    /// `\\[`$"\\]` in a here-document.
    fn heredoc_escape(&mut self, p: usize) -> Option<usize> {
        if self.at(p) == b'\\' && matches!(self.at(p + 1), b'`' | b'$' | b'"' | b'\\') {
            return self.none(p, p + 2, k::BACKTICK_ESCAPE);
        }
        None
    }

    // ---- expansions and strings

    /// `expansion-and-string`.
    fn eas(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        match c {
            b'"' | b'\'' => self.string(p),
            b'$' => {
                if let Some(e) = self.string(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_parameter(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_arithmetic(p) {
                    return Some(e);
                }
                self.expansion_command(p)
            }
            b'?' | b'*' | b'+' | b'@' | b'!' | b'[' => self.expansion_pattern(p),
            b'{' => self.push(p, p + 1, k::BRACE_BEGIN, &[BRACE_BODY]),
            b'`' => self.expansion_command(p),
            b'~' => self.none(p, p + 1, k::TILDE),
            b'%' => self.expansion_job(p),
            _ => None,
        }
    }

    /// `string`.
    fn string(&mut self, p: usize) -> Option<usize> {
        match self.at(p) {
            b'"' => self.push(p, p + 1, k::STR_BEGIN, &[STRING_DOUBLE]),
            b'\'' => self.push(p, p + 1, k::STR_BEGIN, &[STRING_SINGLE]),
            b'$' if self.at(p + 1) == b'\'' => self.push(p, p + 2, k::STR_BEGIN, &[STRING_ANSI_C]),
            b'$' if self.at(p + 1) == b'"' => self.push(p, p + 2, k::STR_BEGIN, &[STRING_LOCALE]),
            _ => None,
        }
    }

    /// `expansion-pattern`.
    fn expansion_pattern(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        if matches!(c, b'?' | b'*' | b'+' | b'@' | b'!') && self.at(p + 1) == b'(' {
            let caps = [(p, p + 1, k::QUANTIFIER), (p + 1, p + 2, k::PARENS_BEGIN)];
            return self.exec(p, p + 2, PLAIN, &caps, Op::Push(&[EXTGLOB]), 0);
        }
        if matches!(c, b'*' | b'?') {
            return self.none(p, p + 1, k::QUANTIFIER);
        }
        if c == b'[' && self.bracket_after(p) {
            return self.push(p, p + 1, k::SET_BEGIN, &[SET_START]);
        }
        None
    }

    fn set_rest(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        // `(?:-)?(\])`
        if c == b']' {
            return self.exec(p, p + 1, PLAIN, &[(p, p + 1, k::SET_END)], Op::Pop, 0);
        }
        if c == b'-' && self.at(p + 1) == b']' {
            return self.exec(p, p + 2, PLAIN, &[(p + 1, p + 2, k::SET_END)], Op::Pop, 0);
        }
        if c == b'-' {
            return self.none(p, p + 1, k::WORD_OP);
        }
        // `(\.)[[:word:]](\.)`, `(=)[[:word:]](=)`
        for (d, begin, end) in [
            (b'.', k::COLLATE_BEGIN, k::COLLATE_END),
            (b'=', k::EQUIVALENCE_BEGIN, k::EQUIVALENCE_END),
        ] {
            if c == d && self.word_at(p + 1) {
                let e = p + 1 + char_len(self.at(p + 1));
                if self.at(e) == d {
                    let caps = [(p, p + 1, begin), (e, e + 1, end)];
                    return self.exec(p, e + 1, PLAIN, &caps, Op::None, 0);
                }
            }
        }
        // `(:)[[:lower:]]+(:)`
        if c == b':' {
            let mut e = p + 1;
            while self.at(e).is_ascii_lowercase() {
                e += 1;
            }
            if e > p + 1 && self.at(e) == b':' {
                let caps = [(p, p + 1, k::CLASS_BEGIN), (e, e + 1, k::CLASS_END)];
                return self.exec(p, e + 1, PLAIN, &caps, Op::None, 0);
            }
        }
        // `\[(?![\.=:])`
        if c == b'[' && !matches!(self.at(p + 1), b'.' | b'=' | b':') {
            return self.none(p, p + 1, PLAIN);
        }
        self.eas(p)
    }

    /// `expansion-parameter`, at `$`.
    fn expansion_parameter(&mut self, p: usize) -> Option<usize> {
        if self.at(p) != b'$' {
            return None;
        }
        let c = self.at(p + 1);
        let mark = (p, p + 1, k::VAR_MARK);
        if c == b'{' {
            let caps = [mark, (p + 1, p + 2, k::PARAMETER_BEGIN)];
            return self.exec(p, p + 2, PLAIN, &caps, Op::Push(&[PARAMETER_START]), 0);
        }
        if c.is_ascii_digit() {
            let caps = [mark, (p + 1, p + 2, k::READWRITE)];
            return self.exec(p, p + 2, PLAIN, &caps, Op::None, 0);
        }
        if matches!(
            c,
            b'$' | b'#' | b'@' | b'!' | b'~' | b'*' | b'?' | b'_' | b'-'
        ) && !self.word_at(p + 2)
        {
            let caps = [mark, (p + 1, p + 2, k::LANG_VAR)];
            return self.exec(p, p + 2, PLAIN, &caps, Op::None, 0);
        }
        if p + 1 < self.b.len()
            && let Some(e) = self.identifier(p + 1)
        {
            let caps = [mark, (p + 1, e, k::READWRITE)];
            return self.exec(p, e, PLAIN, &caps, Op::None, 0);
        }
        None
    }

    /// `expansion-arithmetic`: `(\$)(\(\()(?=.+\)\))`
    fn expansion_arithmetic(&mut self, p: usize) -> Option<usize> {
        if self.starts(p, b"$((") && self.close_from(p + 4) {
            let caps = [(p, p + 1, k::VAR_MARK), (p + 1, p + 3, k::PARENS_BEGIN)];
            return self.exec(p, p + 3, PLAIN, &caps, Op::Push(&[ARITHMETIC_EXPANSION]), 0);
        }
        None
    }

    /// `expansion-command`: `(\$)(\()` and `` ` ``.
    fn expansion_command(&mut self, p: usize) -> Option<usize> {
        if self.starts(p, b"$(") {
            let caps = [(p, p + 1, k::VAR_MARK), (p + 1, p + 2, k::PARENS_BEGIN)];
            return self.exec(p, p + 2, PLAIN, &caps, Op::Push(&[COMMAND_PARENS]), 0);
        }
        if self.at(p) == b'`' {
            return self.push(p, p + 1, k::GROUP_BEGIN, &[BACKTICKS]);
        }
        None
    }

    /// `expansion-job`, at `%`.
    fn expansion_job(&mut self, p: usize) -> Option<usize> {
        let j = p + 1;
        let mark = (p, j, k::JOB_MARK);
        let c = self.at(j);
        if matches!(c, b'%' | b'+' | b'-') {
            return self.exec(
                p,
                j + 1,
                PLAIN,
                &[mark, (j, j + 1, k::JOB_VAR)],
                Op::None,
                0,
            );
        }
        if c.is_ascii_digit() {
            let e = self.digits_end(j);
            return self.exec(p, e, PLAIN, &[mark, (j, e, k::JOB_NUMBER)], Op::None, 0);
        }
        let q = if c == b'?' { j + 1 } else { j };
        let mut e = q;
        while self.word_at(e) {
            e += char_len(self.at(e));
        }
        if e > q {
            let caps = [mark, (j, q, k::QUANTIFIER), (q, e, k::READWRITE)];
            return self.exec(p, e, PLAIN, &caps, Op::None, 0);
        }
        self.none(p, j, k::JOB_MARK)
    }

    /// `expansion-parameter-post-first-character`.
    fn parameter_post(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.parameter_common(p) {
            return Some(e);
        }
        let c = self.at(p);
        let d = self.at(p + 1);
        if p >= self.b.len() {
            return None;
        }
        // `(?=[@*]?/)`
        if c == b'/' || (matches!(c, b'@' | b'*') && d == b'/') {
            return self.set(p, p, PLAIN, &[PARAMETER_SLASH]);
        }
        // `(?=\:?[-+=?])`
        if matches!(c, b'-' | b'+' | b'=' | b'?')
            || (c == b':' && matches!(d, b'-' | b'+' | b'=' | b'?'))
        {
            return self.set(p, p, PLAIN, &[PARAMETER_OP]);
        }
        // `(?=@?:)`
        if c == b':' || (c == b'@' && d == b':') {
            return self.set(p, p, PLAIN, &[PARAMETER_SUBSTRING]);
        }
        // `\#(?=})`
        if c == b'#' && d == b'}' {
            return self.none(p, p + 1, PLAIN);
        }
        // `([@*])?(\#\#?|%%?|\^\^?|,,?)`
        let (s, o) = if matches!(c, b'@' | b'*') && matches!(d, b'#' | b'%' | b'^' | b',') {
            (p + 1, d)
        } else {
            (p, c)
        };
        if matches!(o, b'#' | b'%' | b'^' | b',') {
            let e = if self.at(s + 1) == o { s + 2 } else { s + 1 };
            let caps = [(p, s, k::LANG_VAR), (s, e, k::EXPANSION_OP)];
            return self.exec(p, e, PLAIN, &caps, Op::Set(&[PARAMETER_PATTERN]), 0);
        }
        // `([@*]?)(@)([QEPAa])(?=})`
        let a = if matches!(c, b'@' | b'*') && d == b'@' {
            p + 1
        } else {
            p
        };
        if self.at(a) == b'@'
            && matches!(self.at(a + 1), b'Q' | b'E' | b'P' | b'A' | b'a')
            && self.at(a + 2) == b'}'
        {
            let caps = [
                (p, a, k::LANG_VAR),
                (a, a + 1, k::EXPANSION_OP),
                (a + 1, a + 2, k::SWITCH),
            ];
            return self.exec(p, a + 2, PLAIN, &caps, Op::None, 0);
        }
        // `array`
        if c == b'[' {
            return self.push(p, p + 1, k::SECTION_BRACES_BEGIN, &[ARRAY_BODY]);
        }
        // `[*@](?=})`
        if matches!(c, b'*' | b'@') && d == b'}' {
            return self.none(p, p + 1, k::LANG_VAR);
        }
        None
    }

    /// `expansion-parameter-common`.
    fn parameter_common(&mut self, p: usize) -> Option<usize> {
        match self.at(p) {
            b'}' => self.pop(p, p + 1, k::PARAMETER_END),
            b'"' | b'\'' => self.string(p),
            b'$' => {
                if let Some(e) = self.string(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_parameter(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_arithmetic(p) {
                    return Some(e);
                }
                self.expansion_command(p)
            }
            b'`' => self.expansion_command(p),
            b'~' => self.none(p, p + 1, k::TILDE),
            b'\\' => self.any_escape(p),
            _ => None,
        }
    }

    /// `string-quoted-double-escape-character`: `\\[$`"\\]`, and `\\\n`.
    fn dq_escape(&mut self, p: usize) -> Option<usize> {
        if self.at(p) != b'\\' || p >= self.b.len() {
            return None;
        }
        if matches!(self.at(p + 1), b'$' | b'`' | b'"' | b'\\') {
            return self.none(p, p + 2, k::ESCAPE);
        }
        if p + 1 == self.b.len() {
            return self.push(p, self.b.len() + 1, k::ESCAPE, &[ESCAPED_NEWLINE]);
        }
        None
    }

    /// `\\([abfnrtv'"?]|[0-8]{1,3}|x\h{1,8}|c[a-z])`
    fn ansi_escape(&mut self, p: usize) -> Option<usize> {
        if self.at(p) != b'\\' || p >= self.b.len() {
            return None;
        }
        let j = p + 1;
        let run = |s: &Self, from: usize, max: usize, f: fn(u8) -> bool| {
            let mut e = from;
            while e < from + max && e < s.b.len() && f(s.b[e]) {
                e += 1;
            }
            e
        };
        let e = match self.at(j) {
            b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\'' | b'"' | b'?' => j + 1,
            b'0'..=b'8' => run(self, j, 3, |c| matches!(c, b'0'..=b'8')),
            b'x' => {
                let e = run(self, j + 1, 8, |c| c.is_ascii_hexdigit());
                if e == j + 1 {
                    return None;
                }
                e
            }
            b'c' if self.at(j + 1).is_ascii_lowercase() => j + 2,
            _ => return None,
        };
        self.none(p, e, k::ESCAPE)
    }

    // ---- expressions

    /// `expression`.
    fn expression(&mut self, p: usize) -> Option<usize> {
        let c = self.at(p);
        let d = self.at(p + 1);
        if c.is_ascii_digit() && !self.word_before(p) {
            if c == b'0' && matches!(d, b'x' | b'X') {
                return self.push(p, p + 2, k::HEX_BASE, &[HEX]);
            }
            if c == b'0' && matches!(d, b'0'..=b'7') {
                return self.push(p, p + 1, k::OCTAL_BASE, &[OCTAL]);
            }
            let e = self.digits_end(p);
            if self.at(e) == b'#' {
                let mut f = e + 1;
                while f < self.b.len()
                    && (self.b[f].is_ascii_alphanumeric() || matches!(self.b[f], b'@' | b'_'))
                {
                    f += 1;
                }
                if f > e + 1 {
                    return self.exec(
                        p,
                        f,
                        k::OTHER_BASE,
                        &[(p, e + 1, k::OTHER_BASE_MARK)],
                        Op::None,
                        0,
                    );
                }
            }
            return self.none(p, e, k::DECIMAL);
        }
        if p >= self.b.len() {
            return None;
        }
        let e2 = self.at(p + 2);
        // `[*/%+\-&^|]?=|<<=|>>=`
        let assign = match c {
            b'=' => Some(p + 1),
            b'*' | b'/' | b'%' | b'+' | b'-' | b'&' | b'^' | b'|' if d == b'=' => Some(p + 2),
            b'<' | b'>' if d == c && e2 == b'=' => Some(p + 3),
            _ => None,
        };
        if let Some(e) = assign {
            return self.none(p, e, k::ASSIGN);
        }
        // `\+\+?|\-\-?|\*\*?|%|/`
        match c {
            b'+' | b'-' | b'*' => {
                let e = if d == c { p + 2 } else { p + 1 };
                return self.none(p, e, k::ARITH_OP);
            }
            b'%' | b'/' => return self.none(p, p + 1, k::ARITH_OP),
            _ => {}
        }
        // `<[=<]?|>[=>]?|[=!]=|&&|\:|\|\||!`
        let logical = match c {
            b'<' => Some(if matches!(d, b'=' | b'<') {
                p + 2
            } else {
                p + 1
            }),
            b'>' => Some(if matches!(d, b'=' | b'>') {
                p + 2
            } else {
                p + 1
            }),
            b'!' if d == b'=' => Some(p + 2),
            b'&' if d == b'&' => Some(p + 2),
            b'|' if d == b'|' => Some(p + 2),
            b':' | b'!' => Some(p + 1),
            _ => None,
        };
        if let Some(e) = logical {
            return self.none(p, e, k::LOGICAL);
        }
        match c {
            b'&' | b'|' | b'^' | b'~' => self.none(p, p + 1, k::BITWISE),
            b',' | b';' => self.none(p, p + 1, k::SEPARATOR),
            b'?' => self.none(p, p + 1, k::TERNARY),
            b'(' => self.push(p, p + 1, k::PARENS_BEGIN, &[EXPRESSION_PAREN]),
            b'"' | b'\'' => self.string(p),
            b'$' => {
                if let Some(e) = self.string(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_parameter(p) {
                    return Some(e);
                }
                if let Some(e) = self.expansion_arithmetic(p) {
                    return Some(e);
                }
                self.expansion_command(p)
            }
            b'`' => self.expansion_command(p),
            _ => None,
        }
    }

    /// `expression-test`.
    fn expression_test(&mut self, p: usize) -> Option<usize> {
        if let Some(e) = self.eas(p) {
            return Some(e);
        }
        if p >= self.b.len() {
            return None;
        }
        let c = self.b[p];
        let d = self.at(p + 1);
        if c == b'-' {
            // `((-)[aobcdefghknoprstuvwxzGLNORS])(?=\s)`
            if b"aobcdefghknoprstuvwxzGLNORS".contains(&d)
                && ws(self.at(p + 2))
                && p + 1 < self.b.len()
            {
                let caps = [(p, p + 2, k::OPTION), (p, p + 1, k::OPTION_MARK)];
                return self.exec(p, p + 2, PLAIN, &caps, Op::None, 0);
            }
            // `((-)(?:ef|nt|ot|eq|ne|lt|le|gt|ge))(?=\s)`
            const OPS: [&[u8]; 9] = [
                b"ef", b"nt", b"ot", b"eq", b"ne", b"lt", b"le", b"gt", b"ge",
            ];
            if OPS.iter().any(|o| self.starts(p + 1, o)) && ws(self.at(p + 3)) {
                let caps = [(p, p + 3, k::OPTION), (p, p + 1, k::OPTION_MARK)];
                return self.exec(p, p + 3, PLAIN, &caps, Op::None, 0);
            }
        }
        // `(=~)\s*`
        if c == b'=' && d == b'~' {
            let e = self.ws_end(p + 2);
            return self.exec(
                p,
                e,
                PLAIN,
                &[(p, p + 2, k::LOGICAL)],
                Op::Push(&[REGEXP]),
                0,
            );
        }
        // `==?|!=?|<|>|\|\||&&`
        let e = match c {
            b'=' | b'!' => Some(if d == b'=' { p + 2 } else { p + 1 }),
            b'<' | b'>' => Some(p + 1),
            b'|' if d == b'|' => Some(p + 2),
            b'&' if d == b'&' => Some(p + 2),
            _ => None,
        }?;
        self.none(p, e, k::LOGICAL)
    }

    /// `[+\-?]?=` at `i`: where it ends.
    fn varassign_at(&self, i: usize) -> Option<usize> {
        if i >= self.b.len() {
            return None;
        }
        match self.b[i] {
            b'=' => Some(i + 1),
            b'+' | b'-' | b'?' if self.at(i + 1) == b'=' => Some(i + 2),
            _ => None,
        }
    }

    // ---- the state

    #[inline(always)]
    fn load(&mut self, state: &State) {
        let len = usize::from(state.depth).min(KEPT);
        let mut i = 0;
        while i < len {
            let ctx = state.level(i);
            i += 1;
            if usize::from(ctx) >= CONTEXTS {
                break;
            }
            let mut arg = 0u32;
            if let Some(n) = has_arg(ctx) {
                for j in 0..n {
                    arg |= u32::from(state.level(i + j)) << (8 * j);
                }
                i += n;
            }
            self.push_frame(ctx, arg);
        }
    }

    #[inline(always)]
    fn save(&self) -> State {
        // whole frames from the top, as many as fit: a context's byte and its argument's
        let size = |f: &Frame| 1 + has_arg(f.ctx).unwrap_or(0);
        let mut from = self.depth;
        let mut len = 0;
        while from > 1 {
            let n = size(self.at_frame(from - 1));
            if len + n > KEPT {
                break;
            }
            len += n;
            from -= 1;
        }
        let mut state = State {
            depth: len as u8,
            inexact: !self.exact || from > 1,
            ..State::default()
        };
        let mut i = 0;
        let mut put = |c: u8| {
            state.set_level(i, c);
            i += 1;
        };
        for d in from..self.depth {
            let f = self.at_frame(d);
            put(f.ctx);
            match has_arg(f.ctx) {
                Some(1) => put(f.arg as u8),
                Some(_) => f.arg.to_le_bytes().into_iter().for_each(&mut put),
                None => {}
            }
        }
        state
    }
}

/// Whether a context's patterns are `main`'s and closing bytes, none of which match at the
/// line's `\n` or after it.
fn ends_with_main(ctx: u8) -> bool {
    matches!(
        ctx,
        MAIN | SRC_RECIPE
            | SRC_CALL
            | SRC_ASSIGN
            | SRC_EMBED_SQ
            | SRC_EMBED_DQ
            | SRC_EMBED_BT
            | CMD_PAREN
            | CMD_PAREN_BT
            | CMD_BRACE
            | CMD_BRACE_BT
            | CASE_COMMANDS
            | COMMAND_PARENS
            | BACKTICKS
            | PROCESS_SUBSTITUTION
            | COPROC_BRACES
            | FUNCDEF_BODY
            | FUNCDEF_BODY_BT
    )
}

/// Whether, where nothing matched at a space, nothing can until the run of spaces ends: every
/// pattern of the context that can start on a space reads past the whole run first (`\s*`,
/// `\s+`), so it matches from each place in the run or from none. The others step a
/// character.
fn skips_spaces(ctx: u8) -> bool {
    matches!(
        ctx,
        MAIN | SRC_RECIPE
            | SRC_CALL
            | SRC_ASSIGN
            | SRC_EMBED_SQ
            | SRC_EMBED_DQ
            | SRC_EMBED_BT
            | MAIN_POP_AT_END
            | CMD
            | CMD_BT
            | CMD_PAREN
            | CMD_PAREN_BT
            | CMD_BRACE
            | CMD_BRACE_BT
            | CMD_POST
            | LET
            | LET_BT
            | TEST_BODY
            | TEST_BODY_BT
            | TEST_DOUBLE
            | END_OF_OPTIONS
            | END_OF_OPTIONS_BT
            | CASE_WORD
            | CASE_BODY
            | CASE_PATTERNS
            | CASE_COMMANDS
            | FOR_BODY
            | ARITHMETIC_BODY
            | EXPRESSION_PAREN
            | ARITHMETIC_EXPANSION
            | ARRAY_BODY
            | COMMAND_PARENS
            | BACKTICKS
            | PROCESS_SUBSTITUTION
            | COPROC_BRACES
            | FUNCDEF_BODY
            | FUNCDEF_BODY_BT
            | BUILTIN_END_OF_OPTIONS
            | BRACE_BODY
            | EXTGLOB
            | VARDEF_ARRAY
            | VARDEF_ARRAY_INDEX
    )
}

/// A here-document's end word as a context's argument: its hash. The word is an identifier,
/// so a line ends the here-document where the identifier it starts with has the same hash.
fn tag(word: &[u8]) -> u32 {
    hash(word)
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    #[test]
    fn a_command_and_its_arguments() {
        let m = marked(Lang::Bash, &["echo \"hi $USER\" --flag=1 | grep -v x"]);
        assert!(m.contains(&(
            "echo",
            "meta.function-call.shell support.function.echo.shell"
        )));
        assert!(m.contains(&("USER", "variable.other.readwrite.shell")));
        assert!(m.contains(&("grep", "meta.function-call.shell variable.function.shell")));
        assert!(m.contains(&("|", "keyword.operator.logical.pipe.shell")));
    }

    #[test]
    fn the_builtins_match_follows_the_table() {
        for (i, b) in super::BUILTINS.iter().enumerate() {
            assert_eq!(super::builtin(b.name), Some(i));
        }
    }

    #[test]
    fn a_heredoc_ends_at_its_word() {
        let m = marked(Lang::Bash, &["cat <<EOF", "x $y", "EOF", "ls"]);
        assert!(m.contains(&("x ", "string.unquoted.heredoc.shell")));
        assert!(m.contains(&("EOF", "keyword.control.heredoc-token.shell")));
        assert!(m.contains(&("ls", "meta.function-call.shell variable.function.shell")));
    }
}
