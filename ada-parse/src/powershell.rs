//! PowerShell by a scanner of ada's own (ADR-40, speed first), emitting bat's scopes, with bat's
//! `PowerShell` grammar as the reference it is checked against.
//!
//! bat's grammar (39 contexts, 97 patterns, no embeds) is one `main` of ordered patterns that
//! every bracket's context includes after its closer. The scanner matches them **at the place
//! the scan has reached**, which is syntect's leftmost match, dispatching on the byte there to
//! the few patterns that can start on it, in the grammar's order. A word is read once and looked
//! up once in one table ([`lookup`]), whose bits say every pattern it can start: a verb (`Get`
//! before `-`), a keyword, a variable's name after `$`, an attribute's, an operator's after `-`.
//! Runs where nothing can start (indentation, a string's or comment's text) are passed whole.
//!
//! What syntect's machinery decides is followed: the comment docs' `(?:^|\G)` is tried only
//! where a search starts (a line's start, after `<#` or `#`), and a failed search is cached for
//! the rest of the line, across both comments; the line's `\n` ends `#` comments and `#requires`
//! (whose `\s` may take it), and `$` holds before it; a push past syntect's depth limit matches
//! nothing. The lazy and searching patterns (a command's `.+?`, `${…}`'s `[^}]*`, the
//! prefix `…\` of a command and `foo.exe`'s run) are memoized per line, so a line stays linear.
//!
//! A kind is a leaf, bat's scopes for the token with its pattern's own meta scope, under a
//! **wrap**: the string or comment the token sits in, the one scope above it a theme can select
//! (themes key on single scopes; the brackets' `meta.*` scopes are left out). The stack between
//! lines is the contexts open above `main`, up to 28 (the bytes of [`State`]'s levels).
//! A string that `-match`, `-replace`, `-split`, `[regex]` or `-Pattern` takes is colored as
//! .NET's regex (ADR-40, phase 4; [`crate::regex`]): its text is the regex's, and the string's
//! quotes, escapes and variables are the host's, the regex's state a byte above the string.
//! `\w` past ASCII is a letter or a decimal digit (Oniguruma's Latin-1 table also takes
//! `²³¹¼½¾`; not followed), `\p{L}` a letter, `\p{N}` a number, `\s` Unicode's spaces; `(?i)` folds ASCII case only (Oniguruma's `ſ` for `s` isn't followed).

use crate::machine::KEPT as HALF;
use crate::{Out, State};

// The contexts, by what they read; the first ten include `main`.
const MAIN: u8 = 0;
/// `{ … }`, `scriptblock`.
const SCRIPT: u8 = 1;
/// `( … )`, `subexpression`.
const GROUP: u8 = 2;
/// `@( … )`.
const ARRAY: u8 = 3;
/// `$( … )` outside strings.
const SUBEXPR: u8 = 4;
/// `$( … )` in a string, `interpolation`.
const INTERP: u8 = 5;
/// `@{ … }`, `hashtable`: its `key =` first.
const HASH: u8 = 6;
/// `[ … ]`, `type`: a type's name first.
const TYPE: u8 = 7;
/// An attribute's `( … )`: its parameters after `main`.
const ATTR_ARGS: u8 = 8;
/// An attribute, `[CmdletBinding …]`, to its `]`.
const ATTR: u8 = 9;
/// After `function name`, to `{` or `(`.
const FUNCTION: u8 = 10;
/// `# …`, to the line's end.
const LINE: u8 = 11;
/// `#requires …`, to the line's end.
const REQUIRES: u8 = 12;
/// `<# … #>`.
const BLOCK: u8 = 13;
/// `'…'`.
const SINGLE: u8 = 14;
/// `"…"`.
const DOUBLE: u8 = 15;
/// `@"` to `"@`.
const HERE_D: u8 = 16;
/// `@'` to `'@`.
const HERE_S: u8 = 17;
/// After `--%`, to the line's end.
const STOP: u8 = 18;
const CONTEXTS: usize = 19;
/// A regex in the string below (ADR-40, phase 4): this bit, and the regex's state.
const REGEX: u8 = 0x80;
const _: () = assert!(CONTEXTS <= REGEX as usize);

/// The byte that closes each context that includes `main`, 0 for none.
static CLOSE: [u8; CONTEXTS] = [
    0, b'}', b')', b')', b')', b')', b'}', b']', b')', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// The wraps: the string or comment a token is in.
const W_NONE: u8 = 0;
const W_LINE: u8 = 1;
const W_BLOCK: u8 = 2;
const W_SINGLE: u8 = 3;
const W_DOUBLE: u8 = 4;
const W_HERE_S: u8 = 5;
const W_HERE_D: u8 = 6;
/// A regex's tokens in a single-quoted string, then a double-quoted one: their kinds only.
const W_RE_SINGLE: u16 = 7;
const W_RE_DOUBLE: u16 = 8;

/// Each wrap's scope.
static WRAPS: [&str; 7] = [
    "",
    "comment.line.powershell",
    "comment.block.powershell",
    "string.quoted.single.powershell",
    "string.quoted.double.powershell",
    "string.quoted.single.heredoc.powershell",
    "string.quoted.double.heredoc.powershell",
];

/// The wrap each context gives what is in it, `W_NONE` for its parent's.
static OWN_WRAP: [u8; CONTEXTS] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, W_LINE, 0, W_BLOCK, W_SINGLE, W_DOUBLE, W_HERE_D, W_HERE_S, 0,
];

/// A kind is a leaf in its low bits, the wrap above them.
const LEAF_BITS: u16 = 7;

// The leaves: an index into [`LEAVES`].
const PLAIN: u16 = 0;
const BLOCK_BEGIN: u16 = 1;
const BLOCK_END: u16 = 2;
const DOC: u16 = 3;
const DOC_DOT: u16 = 4;
const DOC_KEYWORD: u16 = 5;
const LINE_MARK: u16 = 6;
const REQUIRES_KEYWORD: u16 = 7;
const KEYWORD_OTHER: u16 = 8;
const PARAMETER: u16 = 9;
const STRING_BEGIN: u16 = 10;
const STRING_END: u16 = 11;
const ESCAPE: u16 = 12;
const INVALID_ESCAPE: u16 = 13;
const UNQUOTED: u16 = 14;
const REDIRECTION: u16 = 15;
const COMMAND: u16 = 16;
const CONTROL: u16 = 17;
const MODIFIER: u16 = 18;
const STORAGE_TYPE: u16 = 19;
const CLASS_NAME: u16 = 20;
const USING: u16 = 21;
const FN_META: u16 = 22;
const FN_TYPE: u16 = 23;
const FN_SCOPE: u16 = 24;
const FN_NAME: u16 = 25;
const ATTR_NAME: u16 = 26;
const ATTR_PARAMETER: u16 = 27;
const ASSIGNMENT: u16 = 28;
const HA: u16 = 29;
const HA_BEGIN: u16 = 30;
const HA_NAME: u16 = 31;
const HA_END: u16 = 32;
const HA_EQUALS: u16 = 33;
const HASH_KEYWORD: u16 = 34;
const BRACES_BEGIN: u16 = 35;
const BRACES_END: u16 = 36;
const GROUP_BEGIN: u16 = 37;
const GROUP_END: u16 = 38;
const BRACKET_BEGIN: u16 = 39;
const BRACKET_END: u16 = 40;
const ARRAY_KEYWORD: u16 = 41;
const SUBEXPR_DOLLAR: u16 = 42;
const INTERP_DOLLAR: u16 = 43;
const INTERP_BEGIN: u16 = 44;
const INTERP_END: u16 = 45;
const HEX: u16 = 46;
const INTEGER: u16 = 47;
const OCTAL: u16 = 48;
const COMPARISON: u16 = 49;
const UNARY: u16 = 50;
const LOGICAL: u16 = 51;
const BITWISE: u16 = 52;
const FORMAT: u16 = 53;
const TERMINATOR: u16 = 54;
const OTHER_OPERATOR: u16 = 55;
const RANGE: u16 = 56;
/// The variables' kinds: four leaves each (the name, `$`, a member, a scope or drive).
const V_LANGUAGE: u16 = 57;
const V_SUPPORT: u16 = 61;
const V_AUTOMATIC: u16 = 65;
const V_PREFERENCE: u16 = 69;
const V_READWRITE: u16 = 73;
const NAME: u16 = 0;
const MARK: u16 = 1;
const MEMBER: u16 = 2;
const RW_SCOPE: u16 = 3;
const RW_DRIVE: u16 = 77;
const RW_BRACES_BEGIN: u16 = 78;
const RW_BRACES_END: u16 = 79;
const RW_BARE_BEGIN: u16 = 80;
const RW_BARE_END: u16 = 81;
const RW_KEYWORD: u16 = 82;

/// Each leaf's scopes under the wrap.
static LEAVES: [&str; 83] = [
    "",
    "punctuation.definition.comment.block.begin.powershell",
    "punctuation.definition.comment.block.end.powershell",
    "comment.documentation.embedded.powershell",
    "comment.documentation.embedded.powershell constant.string.documentation.powershell",
    "comment.documentation.embedded.powershell keyword.operator.documentation.powershell",
    "punctuation.definition.comment.powershell",
    "keyword.control.requires.powershell",
    "keyword.other.powershell",
    "variable.parameter.powershell",
    "punctuation.definition.string.begin.powershell",
    "punctuation.definition.string.end.powershell",
    "constant.character.escape.powershell",
    "invalid.character.escape.powershell",
    "string.unquoted.powershell",
    "keyword.operator.redirection.powershell",
    "support.function.powershell",
    "keyword.control.powershell",
    "storage.modifier.powershell",
    "storage.type.powershell",
    "entity.name.function",
    "keyword.control.using.powershell",
    "meta.function.powershell",
    "meta.function.powershell storage.type.powershell",
    "meta.function.powershell storage.modifier.scope.powershell",
    "meta.function.powershell entity.name.function.powershell",
    "support.function.attribute.powershell",
    "variable.parameter.attribute.powershell",
    "keyword.operator.assignment.powershell",
    "meta.hashtable.assignment.powershell",
    "meta.hashtable.assignment.powershell punctuation.definition.string.begin.powershell",
    "meta.hashtable.assignment.powershell variable.other.readwrite.powershell",
    "meta.hashtable.assignment.powershell punctuation.definition.string.end.powershell",
    "meta.hashtable.assignment.powershell keyword.operator.assignment.powershell",
    "keyword.other.hashtable.begin.powershell",
    "punctuation.section.braces.begin.powershell",
    "punctuation.section.braces.end.powershell",
    "punctuation.section.group.begin.powershell",
    "punctuation.section.group.end.powershell",
    "punctuation.section.bracket.begin.powershell",
    "punctuation.section.bracket.end.powershell",
    "keyword.other.array.begin.powershell",
    "keyword.other.substatement.powershell punctuation.definition.subexpression.powershell",
    "keyword.other.substatement.powershell punctuation.definition.substatement.powershell punctuation.section.embedded.substatement.begin.powershell",
    "punctuation.section.group.begin.powershell punctuation.section.embedded.substatement.begin.powershell",
    "punctuation.section.group.end.powershell punctuation.section.embedded.substatement.end.powershell",
    "constant.numeric.hex.powershell",
    "constant.numeric.integer.powershell",
    "constant.numeric.octal.powershell",
    "keyword.operator.comparison.powershell",
    "keyword.operator.unary.powershell",
    "keyword.operator.logical.powershell",
    "keyword.operator.bitwise.powershell",
    "keyword.operator.string-format.powershell",
    "punctuation.terminator.statement.powershell",
    "keyword.operator.other.powershell",
    "keyword.operator.range.powershell",
    // the variables: the name, `$`, a member, (a scope)
    "constant.language.powershell",
    "constant.language.powershell punctuation.definition.variable.powershell",
    "",
    "",
    "support.constant.variable.powershell",
    "support.constant.variable.powershell punctuation.definition.variable.powershell",
    "support.constant.variable.powershell variable.other.member.powershell",
    "",
    "support.variable.automatic.powershell",
    "support.variable.automatic.powershell punctuation.definition.variable.powershell",
    "support.variable.automatic.powershell variable.other.member.powershell",
    "",
    "variable.language.powershell",
    "variable.language.powershell punctuation.definition.variable.powershell",
    "variable.language.powershell variable.other.member.powershell",
    "",
    "variable.other.readwrite.powershell",
    "variable.other.readwrite.powershell punctuation.definition.variable.powershell",
    "variable.other.readwrite.powershell variable.other.member.powershell",
    "variable.other.readwrite.powershell storage.modifier.scope.powershell",
    "variable.other.readwrite.powershell support.variable.drive.powershell",
    "variable.other.readwrite.powershell punctuation.section.braces.begin.powershell",
    "variable.other.readwrite.powershell punctuation.section.braces.end.powershell",
    "variable.other.readwrite.powershell punctuation.section.braces.begin",
    "variable.other.readwrite.powershell punctuation.section.braces.end",
    "variable.other.readwrite.powershell keyword.other.powershell",
];

/// The scopes under `source.powershell` kind `kind` carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    let (wrap, leaf) = (
        usize::from(kind >> LEAF_BITS),
        usize::from(kind & ((1 << LEAF_BITS) - 1)),
    );
    if wrap >= usize::from(W_RE_SINGLE) {
        let host = if wrap == usize::from(W_RE_SINGLE) {
            W_SINGLE
        } else {
            W_DOUBLE
        };
        let mut out = vec![WRAPS[usize::from(host)]];
        out.extend(crate::regex::scopes(leaf as u16));
        return out;
    }
    let (Some(w), Some(l)) = (WRAPS.get(wrap), LEAVES.get(leaf)) else {
        return Vec::new();
    };
    [*w, *l]
        .iter()
        .flat_map(|s| s.split(' '))
        .filter(|s| !s.is_empty())
        .collect()
}

/// How many contexts a line can hold (syntect's limit is below it).
const MAX: usize = 128;
/// How many the state keeps between lines: [`State`]'s levels and withs.
const KEPT: usize = 2 * HALF;
/// syntect's stack limit (with its two bottom contexts, `__start` and `main`): at it, a push's
/// pattern matches nothing, so the patterns after it get their turn.
const SYNTECT_DEPTH: usize = 99;

/// Lex `b`, a line of PowerShell without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        n: b.len(),
        out,
        stack: [MAIN; MAX],
        wraps: [W_NONE; MAX],
        depth: 1,
        exact: state.is_exact(),
        at: 0,
        docs_failed: false,
        docs_at: usize::MAX,
        backslash: usize::MAX,
        prefix: (usize::MAX, 0, usize::MAX, 0),
        exe: (usize::MAX, 0, usize::MAX),
        brace: (usize::MAX, 0),
        word: (usize::MAX, 0, 0),
        regex_at: usize::MAX,
        regex_type: false,
    };
    let n = usize::from(state.depth);
    for i in 0..n {
        let c = if i < HALF {
            state.levels[i]
        } else {
            state.withs[i - HALF]
        };
        s.stack[1 + i] = c;
        s.wraps[1 + i] = match OWN_WRAP.get(usize::from(c)).copied().unwrap_or(W_NONE) {
            W_NONE => s.wraps[i],
            w => w,
        };
    }
    s.depth = 1 + n;
    s.run();
    let n = s.depth - 1;
    let from = n.saturating_sub(KEPT);
    let mut packed = State {
        depth: (n - from) as u8,
        inexact: !s.exact || from > 0,
        ..State::default()
    };
    for (i, &c) in s.stack[1 + from..s.depth].iter().enumerate() {
        if i < HALF {
            packed.levels[i] = c;
        } else {
            packed.withs[i - HALF] = c;
        }
    }
    *state = packed;
}

// A word's classes, as bits: the patterns it can start.
/// `Get` before `-`: `commands`' verbs.
const VERB: u32 = 1;
/// `main`'s first keyword list.
const KEYWORD: u32 = 1 << 1;
/// `foreach`, `where`: the second.
const FOREACH: u32 = 1 << 2;
/// `hidden`, `static`.
const HIDDEN: u32 = 1 << 3;
const CLASS: u32 = 1 << 4;
const USING_WORD: u32 = 1 << 5;
/// `function`, `filter`, `configuration`, `workflow`.
const FUNCTION_WORD: u32 = 1 << 6;
/// `foreach`, `where`, `sort`, `tee` before `-object`.
const OBJECT: u32 = 1 << 7;
/// An attribute's name after `[`.
const ATTRIBUTE: u32 = 1 << 8;
/// An attribute's parameter.
const ATTR_PARAM: u32 = 1 << 9;
/// `$true`, `$false`, `$null`.
const VAR_LANGUAGE: u32 = 1 << 10;
/// `$Host` and the like.
const VAR_SUPPORT: u32 = 1 << 11;
/// `$_`, `$args` and the like.
const VAR_AUTOMATIC: u32 = 1 << 12;
/// `$ErrorActionPreference` and the like.
const VAR_PREFERENCE: u32 = 1 << 13;
/// A variable's scope: `global`, `local`, `private`, `script`, `using`, `workflow`.
const VAR_SCOPE: u32 = 1 << 14;
/// A function's scope: `global`, `local`, `script`, `private`.
const FN_SCOPE_WORD: u32 = 1 << 15;
/// `namespace`, `module` after `using`.
const USING_KIND: u32 = 1 << 16;
// the operators after `-`
const OP_IS: u32 = 1 << 17;
const OP_COMPARISON: u32 = 1 << 18;
const OP_UNARY: u32 = 1 << 19;
const OP_LOGICAL: u32 = 1 << 20;
const OP_BITWISE: u32 = 1 << 21;
const OP_FORMAT: u32 = 1 << 22;
/// Any operator after `-`.
const OPERATOR: u32 = OP_IS | OP_COMPARISON | OP_UNARY | OP_LOGICAL | OP_BITWISE | OP_FORMAT;
/// After `-`, a regex's operator (`match`, `replace`, `split` and their forms) or `pattern`:
/// the string after it is a regex (ADR-40, phase 4), none of bat's classes.
const REGEX_OP: u32 = 1 << 23;

/// Every word a pattern names, lowercase, with its classes.
const ENTRIES: &[(&[u8], u32)] = &[
    // `commands`' verbs
    (b"add", VERB),
    (b"approve", VERB),
    (b"assert", VERB),
    (b"backup", VERB),
    (b"block", VERB),
    (b"build", VERB),
    (b"checkpoint", VERB),
    (b"clear", VERB),
    (b"close", VERB),
    (b"compare", VERB),
    (b"complete", VERB),
    (b"compress", VERB),
    (b"confirm", VERB),
    (b"connect", VERB),
    (b"convert", VERB),
    (b"convertfrom", VERB),
    (b"convertto", VERB),
    (b"copy", VERB),
    (b"debug", VERB),
    (b"deny", VERB),
    (b"deploy", VERB),
    (b"disable", VERB),
    (b"disconnect", VERB),
    (b"dismount", VERB),
    (b"edit", VERB),
    (b"enable", VERB),
    (b"enter", VERB),
    (b"exit", VERB | KEYWORD),
    (b"expand", VERB),
    (b"export", VERB),
    (b"find", VERB),
    (b"format", VERB),
    (b"get", VERB),
    (b"grant", VERB),
    (b"group", VERB),
    (b"hide", VERB),
    (b"import", VERB),
    (b"initialize", VERB),
    (b"install", VERB),
    (b"invoke", VERB),
    (b"join", VERB | OP_UNARY),
    (b"limit", VERB),
    (b"lock", VERB),
    (b"measure", VERB),
    (b"merge", VERB),
    (b"mount", VERB),
    (b"move", VERB),
    (b"new", VERB),
    (b"open", VERB),
    (b"optimize", VERB),
    (b"out", VERB),
    (b"ping", VERB),
    (b"pop", VERB),
    (b"protect", VERB),
    (b"publish", VERB),
    (b"push", VERB),
    (b"read", VERB),
    (b"receive", VERB),
    (b"redo", VERB),
    (b"register", VERB),
    (b"remove", VERB),
    (b"rename", VERB),
    (b"repair", VERB),
    (b"request", VERB),
    (b"reset", VERB),
    (b"resize", VERB),
    (b"resolve", VERB),
    (b"restart", VERB),
    (b"restore", VERB),
    (b"resume", VERB),
    (b"revoke", VERB),
    (b"save", VERB),
    (b"search", VERB),
    (b"select", VERB),
    (b"send", VERB),
    (b"set", VERB),
    (b"show", VERB),
    (b"skip", VERB),
    (b"split", VERB | OP_UNARY | REGEX_OP),
    (b"csplit", REGEX_OP),
    (b"isplit", REGEX_OP),
    (b"pattern", REGEX_OP),
    (b"start", VERB),
    (b"step", VERB),
    (b"stop", VERB),
    (b"submit", VERB),
    (b"suspend", VERB),
    (b"switch", VERB | KEYWORD | VAR_AUTOMATIC),
    (b"sync", VERB),
    (b"test", VERB),
    (b"trace", VERB),
    (b"unblock", VERB),
    (b"undo", VERB),
    (b"uninstall", VERB),
    (b"unlock", VERB),
    (b"unprotect", VERB),
    (b"unpublish", VERB),
    (b"unregister", VERB),
    (b"update", VERB),
    (b"use", VERB),
    (b"wait", VERB),
    (b"watch", VERB),
    (b"write", VERB),
    // `main`'s keywords
    (b"begin", KEYWORD),
    (b"break", KEYWORD),
    (b"catch", KEYWORD),
    (b"clean", KEYWORD),
    (b"continue", KEYWORD),
    (b"data", KEYWORD),
    (b"default", KEYWORD),
    (b"define", KEYWORD),
    (b"do", KEYWORD),
    (b"dynamicparam", KEYWORD),
    (b"else", KEYWORD),
    (b"elseif", KEYWORD),
    (b"end", KEYWORD),
    (b"finally", KEYWORD),
    (b"for", KEYWORD),
    (b"from", KEYWORD),
    (b"if", KEYWORD),
    (b"in", KEYWORD | OP_COMPARISON),
    (b"inlinescript", KEYWORD),
    (b"parallel", KEYWORD),
    (b"param", KEYWORD),
    (b"process", KEYWORD),
    (b"return", KEYWORD),
    (b"sequence", KEYWORD),
    (b"throw", KEYWORD),
    (b"trap", KEYWORD),
    (b"try", KEYWORD),
    (b"until", KEYWORD),
    (b"var", KEYWORD),
    (b"while", KEYWORD),
    (b"foreach", FOREACH | OBJECT | VAR_AUTOMATIC),
    (b"where", FOREACH | OBJECT),
    (b"sort", OBJECT),
    (b"tee", OBJECT),
    (b"hidden", HIDDEN),
    (b"static", HIDDEN),
    (b"class", CLASS),
    (b"using", USING_WORD | VAR_SCOPE),
    (b"namespace", USING_KIND),
    (b"module", USING_KIND),
    (b"function", FUNCTION_WORD),
    (b"filter", FUNCTION_WORD),
    (b"configuration", FUNCTION_WORD),
    (b"workflow", FUNCTION_WORD | VAR_SCOPE),
    (b"global", VAR_SCOPE | FN_SCOPE_WORD),
    (b"local", VAR_SCOPE | FN_SCOPE_WORD),
    (b"private", VAR_SCOPE | FN_SCOPE_WORD),
    (b"script", VAR_SCOPE | FN_SCOPE_WORD),
    // attributes
    (b"cmdletbinding", ATTRIBUTE),
    (b"alias", ATTRIBUTE),
    (b"outputtype", ATTRIBUTE),
    (b"parameter", ATTRIBUTE),
    (b"validatenotnull", ATTRIBUTE),
    (b"validatenotnullorempty", ATTRIBUTE),
    (b"validatecount", ATTRIBUTE),
    (b"validateset", ATTRIBUTE),
    (b"allownull", ATTRIBUTE),
    (b"allowemptycollection", ATTRIBUTE),
    (b"allowemptystring", ATTRIBUTE),
    (b"validatescript", ATTRIBUTE),
    (b"validaterange", ATTRIBUTE),
    (b"validatepattern", ATTRIBUTE),
    (b"validatelength", ATTRIBUTE),
    (b"supportswildcards", ATTRIBUTE),
    (b"mandatory", ATTR_PARAM),
    (b"valuefrompipeline", ATTR_PARAM),
    (b"valuefrompipelinebypropertyname", ATTR_PARAM),
    (b"valuefromremainingarguments", ATTR_PARAM),
    (b"position", ATTR_PARAM),
    (b"parametersetname", ATTR_PARAM),
    (b"defaultparametersetname", ATTR_PARAM),
    (b"supportsshouldprocess", ATTR_PARAM),
    (b"supportspaging", ATTR_PARAM),
    (b"positionalbinding", ATTR_PARAM),
    (b"helpuri", ATTR_PARAM),
    (b"confirmimpact", ATTR_PARAM),
    (b"helpmessage", ATTR_PARAM),
    // variables
    (b"false", VAR_LANGUAGE),
    (b"null", VAR_LANGUAGE),
    (b"true", VAR_LANGUAGE),
    (b"error", VAR_SUPPORT),
    (b"executioncontext", VAR_SUPPORT),
    (b"host", VAR_SUPPORT),
    (b"home", VAR_SUPPORT),
    (b"pid", VAR_SUPPORT),
    (b"pshome", VAR_SUPPORT),
    (b"psversiontable", VAR_SUPPORT),
    (b"shellid", VAR_SUPPORT),
    (b"_", VAR_AUTOMATIC),
    (b"args", VAR_AUTOMATIC),
    (b"consolefilename", VAR_AUTOMATIC),
    (b"event", VAR_AUTOMATIC),
    (b"eventargs", VAR_AUTOMATIC),
    (b"eventsubscriber", VAR_AUTOMATIC),
    (b"input", VAR_AUTOMATIC),
    (b"lastexitcode", VAR_AUTOMATIC),
    (b"matches", VAR_AUTOMATIC),
    (b"myinvocation", VAR_AUTOMATIC),
    (b"nestedpromptlevel", VAR_AUTOMATIC),
    (b"profile", VAR_AUTOMATIC),
    (b"psboundparameters", VAR_AUTOMATIC),
    (b"pscmdlet", VAR_AUTOMATIC),
    (b"psculture", VAR_AUTOMATIC | VAR_PREFERENCE),
    (b"psdebugcontext", VAR_AUTOMATIC | VAR_PREFERENCE),
    (b"psitem", VAR_AUTOMATIC | VAR_PREFERENCE),
    (b"pscommandpath", VAR_AUTOMATIC),
    (b"psscriptroot", VAR_AUTOMATIC),
    (b"psuiculture", VAR_AUTOMATIC),
    (b"pwd", VAR_AUTOMATIC),
    (b"sender", VAR_AUTOMATIC),
    (b"sourceargs", VAR_AUTOMATIC),
    (b"sourceeventargs", VAR_AUTOMATIC),
    (b"stacktrace", VAR_AUTOMATIC),
    (b"this", VAR_AUTOMATIC),
    (b"confirmpreference", VAR_PREFERENCE),
    (b"debugpreference", VAR_PREFERENCE),
    (b"erroractionpreference", VAR_PREFERENCE),
    (b"errorview", VAR_PREFERENCE),
    (b"formatenumerationlimit", VAR_PREFERENCE),
    (b"informationpreference", VAR_PREFERENCE),
    (b"logcommandhealthevent", VAR_PREFERENCE),
    (b"logcommandlifecycleevent", VAR_PREFERENCE),
    (b"logenginehealthevent", VAR_PREFERENCE),
    (b"logenginelifecycleevent", VAR_PREFERENCE),
    (b"logproviderhealthevent", VAR_PREFERENCE),
    (b"logproviderlifecycleevent", VAR_PREFERENCE),
    (b"maximumaliascount", VAR_PREFERENCE),
    (b"maximumdrivecount", VAR_PREFERENCE),
    (b"maximumerrorcount", VAR_PREFERENCE),
    (b"maximumfunctioncount", VAR_PREFERENCE),
    (b"maximumhistorycount", VAR_PREFERENCE),
    (b"maximumvariablecount", VAR_PREFERENCE),
    (b"ofs", VAR_PREFERENCE),
    (b"outputencoding", VAR_PREFERENCE),
    (b"psdefaultparametervalues", VAR_PREFERENCE),
    (b"psemailserver", VAR_PREFERENCE),
    (b"psmoduleautoloadingpreference", VAR_PREFERENCE),
    (b"pssenderinfo", VAR_PREFERENCE),
    (b"pssessionapplicationname", VAR_PREFERENCE),
    (b"pssessionconfigurationname", VAR_PREFERENCE),
    (b"pssessionoption", VAR_PREFERENCE),
    (b"progresspreference", VAR_PREFERENCE),
    (b"verbosepreference", VAR_PREFERENCE),
    (b"warningpreference", VAR_PREFERENCE),
    (b"whatifpreference", VAR_PREFERENCE),
    // the operators after `-`
    (b"is", OP_IS),
    (b"isnot", OP_IS),
    (b"as", OP_IS),
    (b"eq", OP_COMPARISON),
    (b"ieq", OP_COMPARISON),
    (b"ceq", OP_COMPARISON),
    (b"ne", OP_COMPARISON),
    (b"ine", OP_COMPARISON),
    (b"cne", OP_COMPARISON),
    (b"gt", OP_COMPARISON),
    (b"igt", OP_COMPARISON),
    (b"cgt", OP_COMPARISON),
    (b"ge", OP_COMPARISON),
    (b"ige", OP_COMPARISON),
    (b"cge", OP_COMPARISON),
    (b"lt", OP_COMPARISON),
    (b"ilt", OP_COMPARISON),
    (b"clt", OP_COMPARISON),
    (b"le", OP_COMPARISON),
    (b"ile", OP_COMPARISON),
    (b"cle", OP_COMPARISON),
    (b"like", OP_COMPARISON),
    (b"ilike", OP_COMPARISON),
    (b"clike", OP_COMPARISON),
    (b"notlike", OP_COMPARISON),
    (b"inotlike", OP_COMPARISON),
    (b"cnotlike", OP_COMPARISON),
    (b"match", OP_COMPARISON | REGEX_OP),
    (b"imatch", OP_COMPARISON | REGEX_OP),
    (b"cmatch", OP_COMPARISON | REGEX_OP),
    (b"notmatch", OP_COMPARISON | REGEX_OP),
    (b"inotmatch", OP_COMPARISON | REGEX_OP),
    (b"cnotmatch", OP_COMPARISON | REGEX_OP),
    (b"contains", OP_COMPARISON),
    (b"icontains", OP_COMPARISON),
    (b"ccontains", OP_COMPARISON),
    (b"notcontains", OP_COMPARISON),
    (b"inotcontains", OP_COMPARISON),
    (b"cnotcontains", OP_COMPARISON),
    (b"iin", OP_COMPARISON),
    (b"cin", OP_COMPARISON),
    (b"notin", OP_COMPARISON),
    (b"inotin", OP_COMPARISON),
    (b"cnotin", OP_COMPARISON),
    (b"replace", OP_COMPARISON | REGEX_OP),
    (b"ireplace", OP_COMPARISON | REGEX_OP),
    (b"creplace", OP_COMPARISON | REGEX_OP),
    (b"and", OP_LOGICAL),
    (b"or", OP_LOGICAL),
    (b"not", OP_LOGICAL),
    (b"xor", OP_LOGICAL),
    (b"band", OP_BITWISE),
    (b"bor", OP_BITWISE),
    (b"bnot", OP_BITWISE),
    (b"bxor", OP_BITWISE),
    (b"shl", OP_BITWISE),
    (b"shr", OP_BITWISE),
    (b"f", OP_FORMAT),
];

const SIZE: usize = 1024;

/// FNV-1a over the word's bytes, ASCII case folded.
const fn hash(w: &[u8]) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < w.len() {
        h ^= w[i].to_ascii_lowercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    (h as usize) & (SIZE - 1)
}

/// The table: each slot an entry's index plus one, 0 for none; open addressing.
static SLOTS: [u16; SIZE] = {
    let mut t = [0u16; SIZE];
    let mut i = 0;
    while i < ENTRIES.len() {
        let mut h = hash(ENTRIES[i].0);
        while t[h] != 0 {
            h = (h + 1) & (SIZE - 1);
        }
        t[h] = i as u16 + 1;
        i += 1;
    }
    t
};

/// The longest word in the table: longer ones are none of them.
const LONGEST: usize = 31;

/// The classes of word `w`, ASCII case ignored; 0 for none.
fn lookup(w: &[u8]) -> u32 {
    if w.len() > LONGEST {
        return 0;
    }
    let mut h = hash(w);
    loop {
        let i = SLOTS[h];
        if i == 0 {
            return 0;
        }
        let (e, k) = ENTRIES[usize::from(i - 1)];
        if e.eq_ignore_ascii_case(w) {
            return k;
        }
        h = (h + 1) & (SIZE - 1);
    }
}

/// What a byte is to the contexts that include `main`.
const INERT: u8 = 0;
/// `[A-Za-z]`.
const LETTER: u8 = 1;
const DIGIT: u8 = 2;
/// `_`.
const UNDERSCORE: u8 = 3;
/// Past ASCII.
const WIDE: u8 = 4;
/// A byte a pattern starts on.
const PUNCT: u8 = 5;

static CLASS_OF: [u8; 256] = {
    let mut t = [INERT; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b.is_ascii_alphabetic() {
            LETTER
        } else if b.is_ascii_digit() {
            DIGIT
        } else if b == b'_' {
            UNDERSCORE
        } else if b >= 0x80 {
            WIDE
        } else if matches!(
            b,
            b'<' | b'>'
                | b'#'
                | b'$'
                | b'@'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'"'
                | b'\''
                | b'`'
                | b'-'
                | b'+'
                | b'.'
                | b'%'
                | b'?'
                | b'*'
                | b'/'
                | b'='
                | b'!'
                | b'|'
                | b'&'
                | b';'
                | b','
                | b'\\'
                | b':'
        ) {
            PUNCT
        } else {
            INERT
        };
        c += 1;
    }
    t
};

/// `[A-Za-z0-9_]`.
static WORD: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 128 {
        t[c] = (c as u8).is_ascii_alphanumeric() || c == b'_' as usize;
        c += 1;
    }
    t
};

/// The bytes a double-quoted string's text stops at: `$`, `` ` ``, `"`, and a smart quote's
/// first byte. A here-string's are the same.
static DQ_STOP: [bool; 256] = {
    let mut t = [false; 256];
    t[b'$' as usize] = true;
    t[b'`' as usize] = true;
    t[b'"' as usize] = true;
    t[0xe2] = true;
    t
};

/// Where a step goes: the place the scan goes on from, its stack changed or not.
type Step = usize;

struct Scan<'b, 'o, 's> {
    b: &'b [u8],
    n: usize,
    out: &'o mut Out<'s>,
    stack: [u8; MAX],
    /// Each level's wrap: what a token in it sits in.
    wraps: [u8; MAX],
    depth: usize,
    exact: bool,
    /// Where the spans pushed so far end: a context's text fills up to the next token.
    at: usize,
    /// Whether the docs' patterns failed a search on this line (syntect caches it).
    docs_failed: bool,
    /// Where the last comment's first search starts (after `<#` or `#`): the docs' `\G`.
    docs_at: usize,
    /// The line's last `\`, `n` for none; `usize::MAX` until asked.
    backslash: usize,
    /// A command's prefix `(…)*\` over a run of its bytes: the run's `from` and `to`, and the
    /// last `\` in it a command follows, with the command's end (`usize::MAX` for none).
    prefix: (usize, usize, usize, usize),
    /// `foo.exe`'s run of `[A-Za-z0-9_.-]`: `from`, `to`, and where its last `.exe` with a
    /// `\b` after it starts (`usize::MAX` for none).
    exe: (usize, usize, usize),
    /// The first `}` at or after `from`: (`from`, where), `n` for none.
    brace: (usize, usize),
    /// The last word looked up: where it starts, ends, and its classes.
    word: (usize, usize, u32),
    /// Where the last regex's operator or `-Pattern` on the line ends: a string after it, past
    /// spaces, is a regex.
    regex_at: usize,
    /// Whether the line has had the type `[regex]`, for its casts and calls.
    regex_type: bool,
}

impl Scan<'_, '_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    fn wrap(&self) -> u16 {
        u16::from(self.wraps[self.depth - 1]) << LEAF_BITS
    }

    /// Fill the top context's text up to `to`.
    fn fill(&mut self, to: usize) {
        let to = to.min(self.n);
        if to > self.at {
            let w = self.wrap();
            if w != 0 {
                self.out.push(self.at, to, w | PLAIN);
            }
            self.at = to;
        }
    }

    /// Bytes `start..end` are leaf `leaf` in the top context's wrap.
    fn tok(&mut self, start: usize, end: usize, leaf: u16) {
        self.fill(start);
        let end = end.min(self.n);
        if end > start {
            let kind = self.wrap() | leaf;
            if kind != 0 {
                self.out.push(start, end, kind);
            }
            self.at = end;
        }
    }

    /// Whether a push may match: below syntect's limit.
    fn can_push(&self) -> bool {
        self.depth < SYNTECT_DEPTH
    }

    /// Push context `c` at `at`, the text before it filled first.
    fn push(&mut self, c: u8, at: usize) {
        self.fill(at);
        let w = match OWN_WRAP[usize::from(c)] {
            W_NONE => self.wraps[self.depth - 1],
            w => w,
        };
        self.stack[self.depth] = c;
        self.wraps[self.depth] = w;
        self.depth += 1;
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
    }

    /// The line, context by context.
    fn run(&mut self) {
        let mut p = 0;
        loop {
            let next = match self.top() {
                MAIN..=ATTR_ARGS => self.code(p),
                ATTR => self.attr(p),
                FUNCTION => self.function_rest(p),
                LINE => self.line_comment(p),
                REQUIRES => self.requires(p),
                BLOCK => self.block(p),
                SINGLE => self.single(p),
                DOUBLE | HERE_D => self.double(p),
                HERE_S => self.here_single(p),
                STOP => self.stop(p),
                // past the contexts: a regex in a string
                _ => self.in_regex(p),
            };
            match next {
                Some(q) => p = q,
                None => break,
            }
        }
        self.fill(self.n);
    }

    // ---- the contexts that include `main` ----

    /// The top context, one that includes `main`, from `p`: where its stack changed, or
    /// `None` at the line's end.
    fn code(&mut self, mut p: usize) -> Option<Step> {
        let (b, n) = (self.b, self.n);
        let ctx = self.top();
        let close = CLOSE[usize::from(ctx)];
        if p == 0 && n > 0 && space_len(b, 0).is_some() {
            // `function`'s `^\s*+`: nothing else starts at a space
            if let Some(q) = self.function(0) {
                return Some(q);
            }
        }
        loop {
            while p < n && CLASS_OF[usize::from(b[p])] == INERT {
                p += 1;
            }
            if p >= n {
                return None;
            }
            let c = b[p];
            if c == close {
                let leaf = match ctx {
                    SCRIPT | HASH => BRACES_END,
                    TYPE => BRACKET_END,
                    INTERP => INTERP_END,
                    _ => GROUP_END,
                };
                self.tok(p, p + 1, leaf);
                self.pop();
                return Some(p + 1);
            }
            let r = match CLASS_OF[usize::from(c)] {
                LETTER | DIGIT | UNDERSCORE => self.word_at(p, ctx),
                WIDE => self.wide(p, ctx),
                _ => self.punct(p, ctx),
            };
            match r {
                Go::On(q) => p = q,
                Go::Changed(q) => return Some(q),
            }
        }
    }

    /// The patterns at `p`, a word's byte, in context `ctx`.
    fn word_at(&mut self, p: usize, ctx: u8) -> Go {
        let b = self.b;
        let pw = self.word_before(p);
        if ctx == TYPE
            && b[p] != b'_'
            && !b[p].is_ascii_digit()
            && let Some(q) = self.type_name(p)
        {
            return Go::On(q);
        }
        if pw {
            return self.mid_word(p, ctx);
        }
        let e = word_end(b, p);
        if ctx == HASH
            && let Some(q) = self.hash_key(p, p)
        {
            return Go::On(q);
        }
        let c = b[p];
        // `[2-6]>&1`, `[1-6]>`
        if let Some(q) = self.redirect_digit(p) {
            return Go::On(q);
        }
        // `commands`
        if let Some(q) = self.prefixed_command(p) {
            return Go::On(q);
        }
        let bits =
            if c.is_ascii_alphabetic() && e > p && b[p..e].iter().all(u8::is_ascii_alphabetic) {
                self.classes(p, e)
            } else {
                0
            };
        if bits & VERB != 0
            && b.get(e) == Some(&b'-')
            && let Some(q) = self.command_tail(e + 1)
        {
            self.tok(p, q, COMMAND);
            return Go::On(q);
        }
        if bits & OBJECT != 0
            && b.get(e) == Some(&b'-')
            && at_ci(b, e + 1, b"object")
            && !is_word(b, e + 7)
        {
            self.tok(p, e + 7, COMMAND);
            return Go::On(e + 7);
        }
        if p == 0
            && bits & FUNCTION_WORD != 0
            && let Some(q) = self.function(0)
        {
            return Go::Changed(q);
        }
        if bits & USING_WORD != 0
            && let Some(q) = self.using(p, e)
        {
            return Go::On(q);
        }
        if !c.is_ascii_alphabetic()
            && let Some(q) = self.number(p)
        {
            return Go::On(q);
        }
        if matches!(b.get(e), Some(b'.' | b'-'))
            && let Some(q) = self.exe(p)
        {
            return Go::On(q);
        }
        if bits != 0 {
            let prev = if p > 0 { b[p - 1] } else { b'\n' };
            // `(?<!\w|-|\.)`, with `(?!\w)` true of a whole word
            if bits & KEYWORD != 0 && prev != b'-' && prev != b'.' {
                self.tok(p, e, CONTROL);
                return Go::On(e);
            }
            // `(?<!\w|-|[^\)]\.)((?i:(foreach|where)(?!-object)))(?!\w)`
            if bits & FOREACH != 0
                && prev != b'-'
                && !(prev == b'.' && p >= 2 && b[p - 2] != b')')
                && !(b.get(e) == Some(&b'-') && at_ci(b, e + 1, b"object"))
            {
                self.tok(p, e, CONTROL);
                return Go::On(e);
            }
            if bits & HIDDEN != 0 {
                self.tok(p, e, MODIFIER);
                return Go::On(e);
            }
            if bits & CLASS != 0
                && prev != b'-'
                && let Some(q) = self.class(p, e)
            {
                return Go::On(q);
            }
            if ctx == ATTR_ARGS && bits & ATTR_PARAM != 0 {
                return Go::On(self.attr_param(p, e));
            }
        }
        if ctx == TYPE {
            // a type's name may start at the next character (`[2abc]`)
            return Go::On(p + 1);
        }
        Go::On(self.rest_of_word(p, e))
    }

    /// The patterns at `p`, a word's byte with a word's byte before it.
    fn mid_word(&mut self, p: usize, ctx: u8) -> Go {
        if let Some(q) = self.redirect_digit(p) {
            return Go::On(q);
        }
        if let Some(q) = self.prefixed_command(p) {
            return Go::On(q);
        }
        if ctx == TYPE {
            return Go::On(p + char_len(self.b[p]));
        }
        let e = word_end(self.b, p);
        Go::On(self.rest_of_word(p, e))
    }

    /// The rest of a word from `p` to its end `e`, where nothing starts but a redirection
    /// at its last byte: where the scan goes on.
    fn rest_of_word(&mut self, p: usize, e: usize) -> usize {
        if e > p + 1
            && let Some(q) = self.redirect_digit(e - 1)
        {
            return q;
        }
        e
    }

    /// The patterns at `p`, a byte past ASCII.
    fn wide(&mut self, p: usize, ctx: u8) -> Go {
        let b = self.b;
        if b[p] == 0xe2 && b.get(p + 1) == Some(&0x80) {
            match b.get(p + 2) {
                Some(0x98..=0x9b) => return self.open_string(p, 3, SINGLE),
                Some(0x9c..=0x9e) => return self.open_string(p, 3, DOUBLE),
                _ => {}
            }
        }
        let Some(ch) = wide_char(b, p) else {
            return Go::On(p + 1);
        };
        let len = ch.len_utf8();
        if ctx == TYPE
            && (ch.is_alphabetic() || (ch.is_numeric() && !is_nd(ch)))
            && let Some(q) = self.type_name(p)
        {
            return Go::On(q);
        }
        if !is_word(b, p) {
            return Go::On(p + len);
        }
        let pw = self.word_before(p);
        if !pw
            && ctx == HASH
            && let Some(q) = self.hash_key(p, p)
        {
            return Go::On(q);
        }
        if let Some(q) = self.prefixed_command(p) {
            return Go::On(q);
        }
        if ctx == TYPE {
            return Go::On(p + len);
        }
        let e = word_end(b, p);
        Go::On(self.rest_of_word(p, e))
    }

    /// The patterns at `p`, an ASCII byte other than a word's.
    fn punct(&mut self, p: usize, ctx: u8) -> Go {
        let b = self.b;
        let c = b[p];
        let next = b.get(p + 1).copied().unwrap_or(b'\n');
        match c {
            b'<' => {
                if next == b'#' && self.can_push() {
                    self.push(BLOCK, p);
                    self.tok(p, p + 2, BLOCK_BEGIN);
                    self.docs_at = p + 2;
                    return Go::Changed(p + 2);
                }
                let e = if next == b'<' { p + 2 } else { p + 1 };
                self.tok(p, e, REDIRECTION);
                Go::On(e)
            }
            b'>' => {
                let e = if next == b'>' { p + 2 } else { p + 1 };
                self.tok(p, e, REDIRECTION);
                Go::On(e)
            }
            b'#' => {
                let prev = if p > 0 { b[p - 1] } else { b'\n' };
                if !matches!(prev, b'`' | b'\\' | b'-') && self.can_push() {
                    return Go::Changed(self.open_comment(p));
                }
                Go::On(p + 1)
            }
            b'$' => {
                if let Some(q) = self.variable(p, true) {
                    return Go::On(q);
                }
                if next == b'(' && self.can_push() {
                    self.push(SUBEXPR, p);
                    self.tok(p, p + 1, SUBEXPR_DOLLAR);
                    self.tok(p + 1, p + 2, GROUP_BEGIN);
                    return Go::Changed(p + 2);
                }
                Go::On(p + 1)
            }
            b'@' => self.at_sign(p),
            b'(' => {
                if self.can_push() {
                    self.push(GROUP, p);
                    self.tok(p, p + 1, GROUP_BEGIN);
                    return Go::Changed(p + 1);
                }
                Go::On(p + 1)
            }
            b'{' => {
                if self.can_push() {
                    self.push(SCRIPT, p);
                    self.tok(p, p + 1, BRACES_BEGIN);
                    return Go::Changed(p + 1);
                }
                Go::On(p + 1)
            }
            b'[' => {
                if self.can_push() {
                    if let Some(q) = self.attribute(p) {
                        return Go::Changed(q);
                    }
                    self.push(TYPE, p);
                    self.tok(p, p + 1, BRACKET_BEGIN);
                    return Go::Changed(p + 1);
                }
                Go::On(p + 1)
            }
            b'"' | b'\'' => {
                if ctx == HASH
                    && self.word_before(p)
                    && let Some(q) = self.hash_key(p, p + 1)
                {
                    return Go::On(q);
                }
                self.open_string(p, 1, if c == b'"' { DOUBLE } else { SINGLE })
            }
            b'`' => {
                if let Some((q, leaf)) = escape(b, p) {
                    self.tok(p, q, leaf);
                    return Go::On(q);
                }
                self.tok(p, p + 1, OTHER_OPERATOR);
                Go::On(p + 1)
            }
            b'-' => Go::On(self.dash(p)),
            b'+' => {
                if !self.word_before(p)
                    && let Some(q) = self.number(p)
                {
                    return Go::On(q);
                }
                let e = if next == b'=' { p + 2 } else { p + 1 };
                self.tok(p, e, ASSIGNMENT);
                Go::On(e)
            }
            b'.' => Go::On(self.dot(p)),
            b'%' | b'?' => Go::On(self.percent(p)),
            b'*' | b'/' => {
                let e = if next == b'=' { p + 2 } else { p + 1 };
                self.tok(p, e, ASSIGNMENT);
                Go::On(e)
            }
            b'=' => {
                self.tok(p, p + 1, ASSIGNMENT);
                Go::On(p + 1)
            }
            b'!' => {
                self.tok(p, p + 1, UNARY);
                Go::On(p + 1)
            }
            b'|' | b'&' => {
                if next == c {
                    self.tok(p, p + 2, TERMINATOR);
                    return Go::On(p + 2);
                }
                self.tok(p, p + 1, OTHER_OPERATOR);
                Go::On(p + 1)
            }
            b';' => {
                self.tok(p, p + 1, TERMINATOR);
                Go::On(p + 1)
            }
            b',' => {
                self.tok(p, p + 1, OTHER_OPERATOR);
                Go::On(p + 1)
            }
            b'\\' | b':' => {
                if let Some(q) = self.prefixed_command(p) {
                    return Go::On(q);
                }
                Go::On(p + 1)
            }
            // a closer not this context's
            _ => Go::On(p + 1),
        }
    }

    /// At `p`, `-`: where the scan goes on.
    fn dash(&mut self, p: usize) -> usize {
        let b = self.b;
        if let Some(q) = self.prefixed_command(p) {
            return q;
        }
        let pw = self.word_before(p);
        if !pw {
            if let Some(q) = self.number(p) {
                return q;
            }
        } else if let Some(q) = self.exe(p) {
            return q;
        }
        let next = b.get(p + 1).copied().unwrap_or(b'\n');
        if !pw {
            // `(?<!\w)(--%)(?!\w)`
            if next == b'-' && b.get(p + 2) == Some(&b'%') && !is_word(b, p + 3) && self.can_push()
            {
                self.push(STOP, p);
                self.tok(p, p + 3, CONTROL);
                // `.+`, then the pop at `$`
                self.tok(p + 3, self.n, UNQUOTED);
                self.pop();
                return self.n;
            }
            if next.is_ascii_alphabetic() {
                let mut e = p + 1;
                while e < self.n && b[e].is_ascii_alphabetic() {
                    e += 1;
                }
                // `(?!\p{L})`: the letters run whole
                if !(e < self.n && b[e] >= 0x80 && is_letter(b, e)) {
                    let bits = self.classes(p + 1, e);
                    if bits & REGEX_OP != 0 {
                        self.regex_operator(p, e);
                    }
                    let bits = bits & OPERATOR;
                    let leaf = if bits & OP_IS != 0 {
                        // `\b` after
                        (!is_word(b, e)).then_some(COMPARISON)
                    } else if bits & OP_COMPARISON != 0 {
                        Some(COMPARISON)
                    } else if bits & OP_UNARY != 0 {
                        Some(UNARY)
                    } else if bits & OP_LOGICAL != 0 {
                        Some(LOGICAL)
                    } else if bits & OP_BITWISE != 0 {
                        Some(BITWISE)
                    } else if bits & OP_FORMAT != 0 {
                        Some(FORMAT)
                    } else {
                        None
                    };
                    if let Some(leaf) = leaf {
                        self.tok(p, e, leaf);
                        return e;
                    }
                }
            }
        }
        let e = if next == b'=' { p + 2 } else { p + 1 };
        self.tok(p, e, ASSIGNMENT);
        e
    }

    /// At `p`, `.`: where the scan goes on.
    fn dot(&mut self, p: usize) -> usize {
        let b = self.b;
        let pw = self.word_before(p);
        if !pw {
            if let Some(q) = self.number(p) {
                return q;
            }
        } else if let Some(q) = self.exe(p) {
            return q;
        }
        let next = b.get(p + 1).copied();
        // `(?<!\w)\.(?= )`
        if !pw && next == Some(b' ') {
            self.tok(p, p + 1, OTHER_OPERATOR);
            return p + 1;
        }
        // `(?<!\s|^)\.\.(?=\-?\d|\(|\$)`
        if p > 0 && next == Some(b'.') && !self.space_before(p) {
            let q = p + 2;
            let ok = match b.get(q) {
                Some(b'(' | b'$') => true,
                Some(b'-') => digit(b, q + 1).is_some(),
                _ => digit(b, q).is_some(),
            };
            if ok {
                self.tok(p, q, RANGE);
                return q;
            }
        }
        p + 1
    }

    /// At `p`, `%` or `?`: where the scan goes on.
    fn percent(&mut self, p: usize) -> usize {
        let b = self.b;
        let pw = self.word_before(p);
        let prev = if p > 0 { b[p - 1] } else { b'\n' };
        let after_word = is_word(b, p + 1);
        if !pw && !after_word {
            // `(?<!\w|-|\.)(%|\?)(?!\w)`, then `(?<!\w|-|[^\)]\.)`
            if (prev != b'-' && prev != b'.') || (prev == b'.' && (p < 2 || b[p - 2] == b')')) {
                self.tok(p, p + 1, CONTROL);
                return p + 1;
            }
        }
        if !pw
            && prev != b'-'
            && let Some(q) = self.class(p, p + 1)
        {
            return q;
        }
        if b[p] == b'%' {
            let e = if b.get(p + 1) == Some(&b'=') {
                p + 2
            } else {
                p + 1
            };
            self.tok(p, e, ASSIGNMENT);
            return e;
        }
        p + 1
    }

    /// At `p`, `@`.
    fn at_sign(&mut self, p: usize) -> Go {
        let b = self.b;
        if let Some(q) = self.variable(p, true) {
            return Go::On(q);
        }
        let next = b.get(p + 1).copied().unwrap_or(b'\n');
        if !self.can_push() {
            return Go::On(p + 1);
        }
        if next == b'{' {
            self.push(HASH, p);
            self.tok(p, p + 1, HASH_KEYWORD);
            self.tok(p + 1, p + 2, BRACES_BEGIN);
            return Go::Changed(p + 2);
        }
        // `(@["“-„])\s*$`, `(@['‘-‛])\s*$`
        let quote = match next {
            b'"' => Some((1, HERE_D)),
            b'\'' => Some((1, HERE_S)),
            0xe2 if b.get(p + 2) == Some(&0x80) => match b.get(p + 3) {
                Some(0x9c..=0x9e) => Some((3, HERE_D)),
                Some(0x98..=0x9b) => Some((3, HERE_S)),
                _ => None,
            },
            _ => None,
        };
        if let Some((len, ctx)) = quote {
            let q = p + 1 + len;
            if self.spaces_to_end(q) {
                self.push(ctx, p);
                self.tok(p, q, STRING_BEGIN);
                self.fill(self.n);
                return Go::Changed(self.n);
            }
        }
        if next == b'(' {
            self.push(ARRAY, p);
            self.tok(p, p + 1, ARRAY_KEYWORD);
            self.tok(p + 1, p + 2, GROUP_BEGIN);
            return Go::Changed(p + 2);
        }
        Go::On(p + 1)
    }

    /// A string's opening quote at `p`, `len` bytes, pushing `ctx`.
    fn open_string(&mut self, p: usize, len: usize, ctx: u8) -> Go {
        if !self.can_push() {
            return Go::On(p + len);
        }
        self.push(ctx, p);
        self.tok(p, p + len, STRING_BEGIN);
        self.regex_in(p);
        Go::Changed(p + len)
    }

    /// A string opened at `p`: a regex in it where an operator or a call makes it one: right
    /// after a regex's operator or `-Pattern` (marked where the scan read it), after
    /// `Select-String` or `sls`, and on a line with the type `[regex]`, its casts and calls.
    fn regex_in(&mut self, p: usize) {
        let b = self.b;
        let q = blank_back(b, p);
        if q == 0 {
            return;
        }
        let regex = q == self.regex_at
            || match b[q - 1] {
                b']' | b'(' | b',' => self.regex_type && regex_typed(b, q),
                c => {
                    matches!(c | 0x20, b'g' | b's')
                        && (ends_word_ci(b, q, b"select-string") || ends_word_ci(b, q, b"sls"))
                }
            };
        if regex && self.can_push() {
            let w = self.wraps[self.depth - 1];
            self.stack[self.depth] = REGEX;
            self.wraps[self.depth] = w;
            self.depth += 1;
        }
    }

    /// A regex's operator or `-Pattern` from `p`, its `-`, to `e`: a string after it is a regex,
    /// but after a `-split` with no operand before it, which splits on spaces.
    #[cold]
    fn regex_operator(&mut self, p: usize, e: usize) {
        let b = self.b;
        if b[e - 5..e].eq_ignore_ascii_case(b"split") {
            let v = blank_back(b, p);
            let binary = v > 0
                && (WORD[usize::from(b[v - 1])]
                    || b[v - 1] >= 0x80
                    || matches!(b[v - 1], b')' | b']' | b'}' | b'\'' | b'"' | b'$'));
            if !binary {
                return;
            }
        }
        self.regex_at = e;
    }

    /// In a regex in a string, at `p`: the string's patterns (its end and `''` or `""`, in
    /// `"…"` its escapes and variables) where they match, else the regex to the next place
    /// they can start.
    #[cold]
    #[inline(never)]
    fn in_regex(&mut self, mut p: usize) -> Option<Step> {
        let double = self.stack[self.depth - 2] != SINGLE;
        let b = self.b;
        loop {
            if p >= self.n {
                return None;
            }
            if double {
                match b[p] {
                    b'$' => {
                        if let Some(q) = self.variable(p, false) {
                            p = q;
                            continue;
                        }
                        if b.get(p + 1) == Some(&b'(') && self.can_push() {
                            self.push(INTERP, p);
                            self.tok(p, p + 1, INTERP_DOLLAR);
                            self.tok(p + 1, p + 2, INTERP_BEGIN);
                            return Some(p + 2);
                        }
                    }
                    b'`' => {
                        if let Some((q, leaf)) = escape(b, p) {
                            self.tok(p, q, leaf);
                            p = q;
                            continue;
                        }
                        if self.spaces_to_end(p + 1) {
                            self.tok(p, self.n, KEYWORD_OTHER);
                            return None;
                        }
                    }
                    _ => {
                        if let Some(l) = double_quote(b, p) {
                            if let Some(l2) = double_quote(b, p + l) {
                                self.tok(p, p + l + l2, ESCAPE);
                                p += l + l2;
                                continue;
                            }
                            self.pop();
                            self.tok(p, p + l, STRING_END);
                            self.pop();
                            return Some(p + l);
                        }
                    }
                }
            } else if let Some(l) = single_quote(b, p) {
                if let Some(l2) = single_quote(b, p + l) {
                    self.tok(p, p + l + l2, ESCAPE);
                    p += l + l2;
                    continue;
                }
                self.pop();
                self.tok(p, p + l, STRING_END);
                self.pop();
                return Some(p + l);
            }
            let w = self.wrap();
            let host = &RE_EMBEDS[usize::from(double)];
            let host = crate::regex::Embed {
                base: if double { W_RE_DOUBLE } else { W_RE_SINGLE } << LEAF_BITS,
                text: w | PLAIN,
                ..*host
            };
            let st = self.top() & crate::regex::NET_STATE;
            let (q, st) = crate::regex::embedded(b, p, st, &host, &mut self.at, self.out)?;
            self.stack[self.depth - 1] = REGEX | st;
            p = q;
        }
    }

    /// `#` at `p` and the `#`s after it, pushing a line comment: where it goes on.
    fn open_comment(&mut self, p: usize) -> usize {
        let b = self.b;
        let mut e = p + 1;
        while e < self.n && b[e] == b'#' {
            e += 1;
        }
        self.push(LINE, p);
        self.tok(p, p + 1, LINE_MARK);
        self.fill(e);
        // the search after the push starts here: the docs' `\G`
        self.docs_at = e;
        e
    }

    // ---- words and their patterns ----

    /// The classes of the word `p..e`, its last lookup kept.
    fn classes(&mut self, p: usize, e: usize) -> u32 {
        if self.word.0 == p && self.word.1 == e {
            return self.word.2;
        }
        let bits = lookup(&self.b[p..e]);
        self.word = (p, e, bits);
        bits
    }

    /// Whether the character before `p` is a word's.
    fn word_before(&self, p: usize) -> bool {
        if p == 0 {
            return false;
        }
        let c = self.b[p - 1];
        if c < 0x80 {
            return WORD[usize::from(c)];
        }
        let mut s = p - 1;
        while s > 0 && self.b[s] & 0xc0 == 0x80 {
            s -= 1;
        }
        is_word(self.b, s)
    }

    /// Whether the character before `p` is a space, `\s`.
    fn space_before(&self, p: usize) -> bool {
        let c = self.b[p - 1];
        if c < 0x80 {
            return matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r');
        }
        let mut s = p - 1;
        while s > 0 && self.b[s] & 0xc0 == 0x80 {
            s -= 1;
        }
        space_len(self.b, s) == Some(p - s)
    }

    /// Whether only spaces follow `p` to the line's end.
    fn spaces_to_end(&self, mut p: usize) -> bool {
        while p < self.n {
            match space_len(self.b, p) {
                Some(k) => p += k,
                None => return false,
            }
        }
        true
    }

    /// `[2-6]>&1|[1-6]>` at `p`.
    fn redirect_digit(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if !(b'1'..=b'6').contains(&b[p]) || b.get(p + 1) != Some(&b'>') {
            return None;
        }
        let e = if b[p] != b'1' && b.get(p + 2) == Some(&b'&') && b.get(p + 3) == Some(&b'1') {
            p + 4
        } else {
            p + 2
        };
        self.tok(p, e, REDIRECTION);
        Some(e)
    }

    /// `commands`' first pattern with its prefix, `(?:(\p{L}|\d|_|-|\\|\:)*\\)?\b(?i:Verb)\-…`,
    /// at `p`, a byte of the prefix, when a `\` follows: where it ends.
    fn prefixed_command(&mut self, p: usize) -> Option<usize> {
        if self.backslash == usize::MAX {
            self.backslash = self.b.iter().rposition(|&c| c == b'\\').unwrap_or(self.n);
        }
        if self.backslash == self.n || self.backslash < p {
            return None;
        }
        let (from, to, s, end) = self.prefix;
        let (s, end) = if from <= p && p < to {
            (s, end)
        } else {
            let to = prefix_run(self.b, p);
            let (s, end) = self.last_prefixed(p, to);
            self.prefix = (p, to, s, end);
            (s, end)
        };
        if s == usize::MAX || s < p {
            return None;
        }
        self.tok(p, end, COMMAND);
        Some(end)
    }

    /// The last `\` in the run `p..to` that a command follows, and the command's end.
    fn last_prefixed(&mut self, p: usize, to: usize) -> (usize, usize) {
        let b = self.b;
        let mut s = to;
        while s > p {
            s -= 1;
            if b[s] != b'\\' {
                continue;
            }
            let v = s + 1;
            let e = alpha_end(b, v);
            if e > v
                && b.get(e) == Some(&b'-')
                && !is_word(b, e)
                && lookup(&b[v..e]) & VERB != 0
                && let Some(q) = self.command_tail(e + 1)
            {
                return (s, q);
            }
        }
        (usize::MAX, 0)
    }

    /// A command's `.+?(?:\.(?i:exe|cmd|bat|ps1))?\b` from `k`, after its `-`: where it ends.
    fn command_tail(&self, k: usize) -> Option<usize> {
        let (b, n) = (self.b, self.n);
        if k >= n {
            return None;
        }
        let mut j = k + char_len(b[k]);
        loop {
            if j > n {
                return None;
            }
            if j < n
                && b[j] == b'.'
                && (at_ci(b, j + 1, b"exe")
                    || at_ci(b, j + 1, b"cmd")
                    || at_ci(b, j + 1, b"bat")
                    || at_ci(b, j + 1, b"ps1"))
                && boundary(b, j + 4)
            {
                return Some(j + 4);
            }
            if boundary(b, j) {
                return Some(j);
            }
            if j == n {
                return None;
            }
            j += char_len(b[j]);
        }
    }

    /// `(\b(([A-Za-z0-9\-_\.]+)\.(?i:exe|com|cmd|bat))\b)` at `p`, where `\b` holds.
    fn exe(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let (from, to, d) = self.exe;
        let d = if from <= p && p < to {
            d
        } else {
            // the run, and its last `.exe` with `\b` after
            let mut to = p;
            while to < self.n && (WORD[usize::from(b[to])] || b[to] == b'.' || b[to] == b'-') {
                to += 1;
            }
            let mut d = usize::MAX;
            let mut i = to.saturating_sub(4);
            while i > p {
                if b[i] == b'.'
                    && (at_ci(b, i + 1, b"exe")
                        || at_ci(b, i + 1, b"com")
                        || at_ci(b, i + 1, b"cmd")
                        || at_ci(b, i + 1, b"bat"))
                    && !is_word(b, i + 4)
                {
                    d = i;
                    break;
                }
                i -= 1;
            }
            self.exe = (p, to, d);
            d
        };
        if d == usize::MAX || d <= p {
            return None;
        }
        self.tok(p, d + 4, COMMAND);
        Some(d + 4)
    }

    /// `function`'s pattern at the line's start: pushes [`FUNCTION`].
    fn function(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut k = p;
        while let Some(l) = space_len(b, k) {
            k += l;
        }
        let e = alpha_end(b, k);
        if e == k || is_word(b, e) || lookup(&b[k..e]) & FUNCTION_WORD == 0 {
            return None;
        }
        let mut s = e;
        while let Some(l) = space_len(b, s) {
            s += l;
        }
        if s == e || s >= self.n || !self.can_push() {
            return None;
        }
        // `(?:(global|local|script|private):)?`
        let se = alpha_end(b, s);
        let scope = se > s
            && b.get(se) == Some(&b':')
            && lookup(&b[s..se]) & FN_SCOPE_WORD != 0
            && fn_word_end(b, se + 1) > se + 1;
        let name_at = if scope { se + 1 } else { s };
        let ne = fn_word_end(b, name_at);
        if ne == name_at {
            return None;
        }
        self.tok(p, k, FN_META);
        self.tok(k, e, FN_TYPE);
        self.tok(e, s, FN_META);
        if scope {
            self.tok(s, se, FN_SCOPE);
            self.tok(se, se + 1, FN_META);
        }
        self.tok(name_at, ne, FN_NAME);
        self.push(FUNCTION, ne);
        Some(ne)
    }

    /// `using namespace X` at `p`, its first word ending at `e`.
    fn using(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let mut s = e;
        while let Some(l) = space_len(b, s) {
            s += l;
        }
        if s == e {
            return None;
        }
        let ke = alpha_end(b, s);
        if ke == s || lookup(&b[s..ke]) & USING_KIND == 0 {
            return None;
        }
        let mut t = ke;
        while let Some(l) = space_len(b, t) {
            t += l;
        }
        if t == ke || !is_word(b, t) {
            return None;
        }
        // `(?:\w+(?:\.)?)+`
        let mut q = t;
        while is_word(b, q) {
            q = word_end(b, q);
            if b.get(q) == Some(&b'.') {
                q += 1;
            }
        }
        self.tok(p, e, USING);
        self.tok(s, ke, KEYWORD_OTHER);
        self.tok(t, q, PARAMETER);
        Some(q)
    }

    /// `(class|%|\?)(?:\s)+((?:\p{L}|\d|_|-|)+)\b` at `p`, its first token ending at `e`.
    fn class(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        let mut a = e;
        while let Some(l) = space_len(b, a) {
            a += l;
        }
        if a == e {
            return None;
        }
        let mut ne = a;
        loop {
            if ne < self.n && b[ne] == b'-' {
                ne += 1;
            } else if is_word(b, ne) {
                ne += if b[ne] < 0x80 { 1 } else { char_len(b[ne]) };
            } else {
                break;
            }
        }
        // back to the last place `\b` holds
        while !boundary(b, ne) {
            if ne == a {
                return None;
            }
            ne -= 1;
            while ne > a && b[ne] & 0xc0 == 0x80 {
                ne -= 1;
            }
        }
        self.tok(p, e, STORAGE_TYPE);
        self.tok(a, ne, CLASS_NAME);
        Some(ne)
    }

    /// An attribute's parameter `p..e`, then `(?:\s+)?(=)?`.
    fn attr_param(&mut self, p: usize, e: usize) -> usize {
        let b = self.b;
        self.tok(p, e, ATTR_PARAMETER);
        let mut q = e;
        while let Some(l) = space_len(b, q) {
            q += l;
        }
        if b.get(q) == Some(&b'=') {
            self.tok(q, q + 1, ASSIGNMENT);
            q += 1;
        }
        q
    }

    /// `(\[)\s*\b(?i)(cmdletbinding|…)\b` at `p`: pushes [`ATTR`].
    fn attribute(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut s = p + 1;
        while let Some(l) = space_len(b, s) {
            s += l;
        }
        if !b.get(s).is_some_and(u8::is_ascii_alphabetic) {
            return None;
        }
        let e = word_end(b, s);
        if lookup(&b[s..e]) & ATTRIBUTE == 0 {
            return None;
        }
        self.push(ATTR, p);
        self.tok(p, p + 1, BRACKET_BEGIN);
        self.tok(s, e, ATTR_NAME);
        Some(e)
    }

    /// The hashtable's `\b((?:\'|\")?)(\w+)((?:\'|\")?)(?:\s+)?(=)(?:\s+)?` at `p`, its
    /// name starting at `w` (after a quote, or at `p`).
    fn hash_key(&mut self, p: usize, w: usize) -> Option<usize> {
        let b = self.b;
        if !is_word(b, w) {
            return None;
        }
        let e = word_end(b, w);
        let qe = if matches!(b.get(e), Some(b'\'' | b'"')) {
            e + 1
        } else {
            e
        };
        let mut s = qe;
        while let Some(l) = space_len(b, s) {
            s += l;
        }
        if b.get(s) != Some(&b'=') {
            return None;
        }
        let mut t = s + 1;
        while let Some(l) = space_len(b, t) {
            t += l;
        }
        self.tok(p, w, HA_BEGIN);
        self.tok(w, e, HA_NAME);
        self.tok(e, qe, HA_END);
        self.tok(qe, s, HA);
        self.tok(s, s + 1, HA_EQUALS);
        self.tok(s + 1, t, HA);
        Some(t.min(self.n))
    }

    /// The type context's `(?!\d+|\.)(?:\p{L}|\p{N}|\.)+` at `p`.
    fn type_name(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let mut q = p;
        while q < self.n {
            let c = b[q];
            if c.is_ascii_alphanumeric() || c == b'.' {
                q += 1;
            } else if c >= 0x80
                && let Some(ch) = wide_char(b, q)
                && (ch.is_alphabetic() || ch.is_numeric())
            {
                q += ch.len_utf8();
            } else {
                break;
            }
        }
        if q == p {
            return None;
        }
        let name = &b[p..q];
        if name.len() >= 5 && name[name.len() - 5..].eq_ignore_ascii_case(b"regex") {
            self.regex_type |= name.len() == 5 || name[name.len() - 6] == b'.';
        }
        self.tok(p, q, STORAGE_TYPE);
        Some(q)
    }

    /// `numericConstant` at `p`, where `(?<!\w)` holds: where it ends.
    fn number(&mut self, p: usize) -> Option<usize> {
        let (end, unit, leaf) = number(self.b, p)?;
        self.tok(p, unit, leaf);
        self.tok(unit, end, KEYWORD_OTHER);
        Some(end)
    }

    // ---- variables ----

    /// `variable` (with `members`) or `variableNoProperty` at `p`, `$` or `@`.
    fn variable(&mut self, p: usize, members: bool) -> Option<usize> {
        let b = self.b;
        let sigil_at = b[p] == b'@';
        let q = p + 1;
        let c = b.get(q).copied().unwrap_or(b'\n');
        if !sigil_at {
            // `(\$)((?:[$^?])`
            if matches!(c, b'$' | b'^' | b'?') {
                let e = if members { self.members(q + 1) } else { q + 1 };
                self.tok(p, q, V_AUTOMATIC + MARK);
                self.tok(q, q + 1, V_AUTOMATIC + NAME);
                self.tok(q + 1, e, V_AUTOMATIC + MEMBER);
                return Some(e);
            }
            if c == b'{' {
                return self.braced(p, members);
            }
        }
        if !is_word(b, q) {
            return None;
        }
        // the keywords' `\b`: the whole word
        let e = word_end(b, q);
        let bits = if !sigil_at && b[q..e].iter().all(|&c| WORD[usize::from(c)]) {
            self.classes(q, e)
        } else {
            0
        };
        let base = if bits & VAR_LANGUAGE != 0 {
            // no members
            self.tok(p, q, V_LANGUAGE + MARK);
            self.tok(q, e, V_LANGUAGE + NAME);
            return Some(e);
        } else if bits & VAR_SUPPORT != 0 {
            V_SUPPORT
        } else if bits & VAR_AUTOMATIC != 0 {
            V_AUTOMATIC
        } else if bits & VAR_PREFERENCE != 0 {
            V_PREFERENCE
        } else {
            0
        };
        if base != 0 {
            let m = if members { self.members(e) } else { e };
            self.tok(p, q, base + MARK);
            self.tok(q, e, base + NAME);
            self.tok(e, m, base + MEMBER);
            return Some(m);
        }
        // `(\$|@)(global|…):((?:\p{L}|\d|_)+)`
        let we = alpha_end(b, q);
        if we > q
            && b.get(we) == Some(&b':')
            && is_word(b, we + 1)
            && self.classes(q, we) & VAR_SCOPE != 0
        {
            let ne = word_end(b, we + 1);
            let m = if members { self.members(ne) } else { ne };
            self.tok(p, q, V_READWRITE + MARK);
            self.tok(q, we, V_READWRITE + RW_SCOPE);
            self.tok(we, ne, V_READWRITE + NAME);
            self.tok(ne, m, V_READWRITE + MEMBER);
            return Some(m);
        }
        // `(\$|@)((?:\p{L}|\d|_)+:)?((?:\p{L}|\d|_)+)`
        let e = word_end(b, q);
        let (drive, ne) = if b.get(e) == Some(&b':') && is_word(b, e + 1) {
            (e + 1, word_end(b, e + 1))
        } else {
            (q, e)
        };
        let m = if members { self.members(ne) } else { ne };
        self.tok(p, q, V_READWRITE + MARK);
        self.tok(q, drive, RW_DRIVE);
        self.tok(drive, ne, V_READWRITE + NAME);
        self.tok(ne, m, V_READWRITE + MEMBER);
        Some(m)
    }

    /// `${…}` at `p`: `(\$)(\{)(scope:|drive:)?([^}]*[^}`])(\})`, then members.
    fn braced(&mut self, p: usize, members: bool) -> Option<usize> {
        let b = self.b;
        let open = p + 1;
        let close = self.next_brace(open + 1);
        if close >= self.n || close == open + 1 || b[close - 1] == b'`' {
            return None;
        }
        let c0 = open + 1;
        let we = alpha_end(b, c0);
        // the scope form needs a scope word, `:`, and text after it
        let scoped = we > c0
            && b.get(we) == Some(&b':')
            && we + 1 < close
            && b[close - 1] != b'`'
            && lookup(&b[c0..we]) & VAR_SCOPE != 0;
        let m = if members {
            self.members(close + 1)
        } else {
            close + 1
        };
        self.tok(p, open, V_READWRITE + MARK);
        if scoped {
            if members {
                self.tok(open, c0, RW_BRACES_BEGIN);
                self.tok(c0, we, V_READWRITE + RW_SCOPE);
                self.tok(we, close, V_READWRITE + NAME);
                self.tok(close, close + 1, RW_BRACES_END);
            } else {
                self.tok(open, c0, V_READWRITE + RW_SCOPE);
                self.tok(c0, we + 1, V_READWRITE + NAME);
                self.tok(we + 1, close, RW_KEYWORD);
                self.tok(close, close + 1, V_READWRITE + MEMBER);
            }
        } else {
            // `((?:\p{L}|\d|_)+:)?`, with text after it
            let de = word_end(b, c0);
            let drive = de > c0 && b.get(de) == Some(&b':') && de + 1 < close;
            let (open_leaf, close_leaf) = if members {
                (RW_BRACES_BEGIN, RW_BRACES_END)
            } else {
                (RW_BARE_BEGIN, RW_BARE_END)
            };
            self.tok(open, c0, open_leaf);
            if drive {
                self.tok(c0, de + 1, RW_DRIVE);
                self.tok(de + 1, close, V_READWRITE + NAME);
            } else {
                self.tok(c0, close, V_READWRITE + NAME);
            }
            self.tok(close, close + 1, close_leaf);
        }
        self.tok(close + 1, m, V_READWRITE + MEMBER);
        Some(m)
    }

    /// The first `}` at or after `from`, `n` for none; kept for the line.
    fn next_brace(&mut self, from: usize) -> usize {
        let (f, at) = self.brace;
        if f <= from && from <= at && f != usize::MAX {
            return at;
        }
        let at = (from..self.n)
            .find(|&i| self.b[i] == b'}')
            .unwrap_or(self.n);
        self.brace = (from, at);
        at
    }

    /// `((?:\.(?:\p{L}|\d|_)+)*\b)?` at `p`: where the members end, the longest run of
    /// them with `\b` after it.
    fn members(&self, p: usize) -> usize {
        let b = self.b;
        let (mut q, mut best) = (p, p);
        while b.get(q) == Some(&b'.') && is_word(b, q + 1) {
            q = word_end(b, q + 1);
            if boundary(b, q) {
                best = q;
            }
        }
        best
    }

    // ---- the other contexts ----

    /// An attribute: `]` pops, `(` pushes its arguments.
    fn attr(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        while p < self.n {
            match b[p] {
                b']' => {
                    self.tok(p, p + 1, BRACKET_END);
                    self.pop();
                    return Some(p + 1);
                }
                b'(' if self.can_push() => {
                    self.push(ATTR_ARGS, p);
                    self.tok(p, p + 1, GROUP_BEGIN);
                    return Some(p + 1);
                }
                _ => p += 1,
            }
        }
        None
    }

    /// After `function name`: `(?=\{|\()` pops; a comment may come first.
    fn function_rest(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        while p < self.n {
            match b[p] {
                b'{' | b'(' => {
                    self.pop();
                    return Some(p);
                }
                b'#' => {
                    let prev = if p > 0 { b[p - 1] } else { b'\n' };
                    if !matches!(prev, b'`' | b'\\' | b'-') && self.can_push() {
                        return Some(self.open_comment(p));
                    }
                    p += 1;
                }
                _ => p += 1,
            }
        }
        None
    }

    /// The docs' patterns, `(?:^|\G)…`, at `p`, where a search starts: where they end.
    fn docs(&mut self, p: usize) -> Option<usize> {
        if self.docs_failed {
            return None;
        }
        let b = self.b;
        let mut s = p;
        while let Some(l) = space_len(b, s) {
            s += l;
        }
        if b.get(s) == Some(&b'.') {
            let k = s + 1;
            let e = alpha_end(b, k);
            let word = &b[k..e];
            if DOCS.iter().any(|w| w.eq_ignore_ascii_case(word)) {
                // `\s*$`
                if self.spaces_to_end(e) {
                    self.tok(p, s, DOC);
                    self.tok(s, k, DOC_DOT);
                    self.tok(k, e, DOC_KEYWORD);
                    self.tok(e, self.n, DOC);
                    return Some(self.n);
                }
            }
            // `(EXTERNALHELP|FORWARDHELP(?:CATEGORY|TARGETNAME)|PARAMETER|REMOTEHELPRUNSPACE)`,
            // `\s+(.+?)\s*$`
            if DOCS_ARG.iter().any(|w| w.eq_ignore_ascii_case(word)) {
                let mut a = e;
                let mut last_space = e;
                while let Some(l) = space_len(b, a) {
                    last_space = a;
                    a += l;
                }
                if a > e {
                    // the argument: to the last byte that isn't a space
                    let mut t = self.n;
                    while t > a {
                        let mut c = t - 1;
                        while c > a && b[c] & 0xc0 == 0x80 {
                            c -= 1;
                        }
                        if space_len(b, c) == Some(t - c) {
                            t = c;
                        } else {
                            break;
                        }
                    }
                    let (a, t) = if t > a {
                        (a, t)
                    } else if last_space > e {
                        // only spaces: `.+?` takes the last of them
                        (last_space, a)
                    } else {
                        (a, a)
                    };
                    if t > a {
                        self.tok(p, s, DOC);
                        self.tok(s, k, DOC_DOT);
                        self.tok(k, e, DOC_KEYWORD);
                        self.tok(e, a, DOC);
                        self.tok(a, t, DOC_KEYWORD);
                        self.tok(t, self.n, DOC);
                        return Some(self.n);
                    }
                }
            }
        }
        self.docs_failed = true;
        None
    }

    /// A line comment from `p`: `#requires`, docs where the search starts, the pop at `$`.
    fn line_comment(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        if p >= self.n {
            self.fill(self.n);
            self.pop();
            return if p > self.n { None } else { Some(self.n) };
        }
        if p == self.docs_at
            && let Some(q) = self.docs(p)
        {
            return Some(q);
        }
        loop {
            // `(?<=#)(?i:(requires))\s`
            if p > 0 && b[p - 1] == b'#' && at_ci(b, p, b"requires") && self.can_push() {
                let e = p + 8;
                let sl = if e >= self.n {
                    Some(1)
                } else {
                    space_len(b, e)
                };
                if let Some(l) = sl {
                    self.push(REQUIRES, p);
                    self.tok(p, (e + l).min(self.n), REQUIRES_KEYWORD);
                    return Some(e + l);
                }
            }
            match b[p..].iter().position(|&c| c == b'#') {
                Some(i) => p += i + 1,
                None => {
                    self.fill(self.n);
                    self.pop();
                    return Some(self.n.max(p));
                }
            }
            if p >= self.n {
                self.fill(self.n);
                self.pop();
                return Some(self.n);
            }
        }
    }

    /// `#requires`'s arguments from `p`, to the pop at `$`.
    fn requires(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        while p < self.n {
            let c = b[p];
            if c == b'-'
                && let Some(e) = REQUIRES_FLAGS
                    .iter()
                    .find(|w| at_ci(b, p + 1, w))
                    .map(|w| p + 1 + w.len())
            {
                self.tok(p, e, KEYWORD_OTHER);
                p = e;
                continue;
            }
            // `(?<!-)\b\p{L}+`
            if (p == 0 || b[p - 1] != b'-') && !self.word_before(p) && is_letter(b, p) {
                let mut e = p;
                while is_letter(b, e) {
                    e += char_len(b[e]);
                }
                self.tok(p, e, PARAMETER);
                p = e;
                continue;
            }
            // `\d+(?:\.\d+)*`
            if let Some(mut e) = digit(b, p) {
                e = digits0(b, e);
                while b.get(e) == Some(&b'.')
                    && let Some(f) = digit(b, e + 1)
                {
                    e = digits0(b, f);
                }
                self.tok(p, e, PARAMETER);
                p = e;
                continue;
            }
            if c == b'@' && b.get(p + 1) == Some(&b'{') && self.can_push() {
                self.push(HASH, p);
                self.tok(p, p + 1, HASH_KEYWORD);
                self.tok(p + 1, p + 2, BRACES_BEGIN);
                return Some(p + 2);
            }
            p += char_len(c);
        }
        self.fill(self.n);
        self.pop();
        Some(self.n.max(p))
    }

    /// A block comment from `p`: `#>` pops; docs where the search starts.
    fn block(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        if p == 0 || p == self.docs_at {
            // `#>` first: a match where the search starts ends it
            if !(b.get(p) == Some(&b'#') && b.get(p + 1) == Some(&b'>'))
                && let Some(q) = self.docs(p)
            {
                return Some(q);
            }
        }
        while p + 1 < self.n {
            match b[p..self.n - 1].iter().position(|&c| c == b'#') {
                Some(i) => p += i,
                None => break,
            }
            if b[p + 1] == b'>' {
                self.tok(p, p + 2, BLOCK_END);
                self.pop();
                return Some(p + 2);
            }
            p += 1;
        }
        None
    }

    /// A single-quoted string from `p`.
    fn single(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        while p < self.n {
            let c = b[p];
            let q = match c {
                b'\'' => 1,
                0xe2 if b.get(p + 1) == Some(&0x80)
                    && matches!(b.get(p + 2), Some(0x98..=0x9b)) =>
                {
                    3
                }
                _ => {
                    p += 1;
                    continue;
                }
            };
            if let Some(l) = single_quote(b, p + q) {
                self.tok(p, p + q + l, ESCAPE);
                p += q + l;
                continue;
            }
            self.tok(p, p + q, STRING_END);
            self.pop();
            return Some(p + q);
        }
        None
    }

    /// A single-quoted here-string from `p`: `^'@` pops.
    fn here_single(&mut self, p: usize) -> Option<Step> {
        let b = self.b;
        if p == 0
            && let Some(l) = single_quote(b, 0)
            && b.get(l) == Some(&b'@')
        {
            self.tok(0, l + 1, STRING_END);
            self.pop();
            return Some(l + 1);
        }
        None
    }

    /// A double-quoted string or here-string from `p`.
    fn double(&mut self, mut p: usize) -> Option<Step> {
        let b = self.b;
        let here = self.top() == HERE_D;
        if here
            && p == 0
            && let Some(l) = double_quote(b, 0)
            && b.get(l) == Some(&b'@')
        {
            self.tok(0, l + 1, STRING_END);
            self.pop();
            return Some(l + 1);
        }
        loop {
            while p < self.n && !DQ_STOP[usize::from(b[p])] {
                p += 1;
            }
            if p >= self.n {
                return None;
            }
            match b[p] {
                b'$' => {
                    if let Some(q) = self.variable(p, false) {
                        p = q;
                        continue;
                    }
                    if b.get(p + 1) == Some(&b'(') && self.can_push() {
                        self.push(INTERP, p);
                        self.tok(p, p + 1, INTERP_DOLLAR);
                        self.tok(p + 1, p + 2, INTERP_BEGIN);
                        return Some(p + 2);
                    }
                    p += 1;
                }
                b'`' => {
                    if let Some((q, leaf)) = escape(b, p) {
                        self.tok(p, q, leaf);
                        p = q;
                    } else if !here && self.spaces_to_end(p + 1) {
                        // `` `\s*$ ``
                        self.tok(p, self.n, KEYWORD_OTHER);
                        return None;
                    } else {
                        p += 1;
                    }
                }
                _ => {
                    let Some(l) = double_quote(b, p) else {
                        p += 1;
                        continue;
                    };
                    if here {
                        p += l;
                        continue;
                    }
                    if let Some(l2) = double_quote(b, p + l) {
                        self.tok(p, p + l + l2, ESCAPE);
                        p += l + l2;
                        continue;
                    }
                    self.tok(p, p + l, STRING_END);
                    self.pop();
                    return Some(p + l);
                }
            }
        }
    }

    /// After `--%`: never open at a line's start, since it pops at the line's end.
    fn stop(&mut self, _p: usize) -> Option<Step> {
        self.pop();
        Some(self.n)
    }
}

/// Where a pattern leaves the scan: on in the same context, or with its stack changed.
enum Go {
    On(usize),
    Changed(usize),
}

/// The docs' first list: `\s*$` after.
const DOCS: &[&[u8]] = &[
    b"component",
    b"description",
    b"example",
    b"functionality",
    b"inputs",
    b"link",
    b"notes",
    b"outputs",
    b"role",
    b"synopsis",
];

/// The docs' second list: an argument after.
const DOCS_ARG: &[&[u8]] = &[
    b"externalhelp",
    b"forwardhelpcategory",
    b"forwardhelptargetname",
    b"parameter",
    b"remotehelprunspace",
];

/// `#requires`' flags after `-`, in the grammar's order: a prefix matches.
const REQUIRES_FLAGS: &[&[u8]] = &[
    b"modules",
    b"pssnapin",
    b"runasadministrator",
    b"shellid",
    b"version",
    b"assembly",
    b"psedition",
];

/// `numericConstant` at `p`: where it ends, where its unit starts, and its leaf.
fn number(b: &[u8], p: usize) -> Option<(usize, usize, u16)> {
    let q = if matches!(b.get(p), Some(b'-' | b'+')) {
        p + 1
    } else {
        p
    };
    let int = |at: usize| {
        let mut e = at;
        while e < b.len() && (b[e].is_ascii_digit() || b[e] == b'_') {
            e += 1;
        }
        e
    };
    let tf = |at: usize| matches!(b.get(at), Some(b'F' | b'f' | b'D' | b'd' | b'M' | b'm'));
    let hex_like = |at: usize, x: &[u8; 2]| {
        b.get(at) == Some(&b'0') && b.get(at + 1).is_some_and(|c| x.contains(c))
    };
    // 1: hex
    if hex_like(q, b"xX") {
        let d = q + 2;
        let mut e = d;
        while e < b.len() && (b[e].is_ascii_hexdigit() || b[e] == b'_') {
            e += 1;
        }
        let mut h = e;
        while h > d {
            if let Some(r) = suffixed_tail(b, h) {
                return Some((r.0, r.1, HEX));
            }
            h -= 1;
        }
    }
    // 2: `(?:[0-9_]+)?\.[0-9_]+(?:[eE][0-9]+)?[FfDdMm]?`
    let i = int(q);
    if b.get(i) == Some(&b'.') {
        let f = int(i + 1);
        if f > i + 1 {
            let mut ends = [usize::MAX; 2];
            if matches!(b.get(f), Some(b'e' | b'E')) {
                let mut x = f + 1;
                while x < b.len() && b[x].is_ascii_digit() {
                    x += 1;
                }
                if x > f + 1 {
                    ends[0] = x;
                }
            }
            ends[1] = f;
            for e in ends {
                if e == usize::MAX {
                    continue;
                }
                if tf(e)
                    && let Some(r) = tail(b, e + 1)
                {
                    return Some((r.0, r.1, INTEGER));
                }
                if let Some(r) = tail(b, e) {
                    return Some((r.0, r.1, INTEGER));
                }
            }
        }
    }
    // 3: binary
    if hex_like(q, b"bB") {
        let d = q + 2;
        let mut e = d;
        while e < b.len() && matches!(b[e], b'0' | b'1' | b'_') {
            e += 1;
        }
        let mut h = e;
        while h > d {
            if let Some(r) = suffixed_tail(b, h) {
                return Some((r.0, r.1, OCTAL));
            }
            h -= 1;
        }
    }
    if i == q {
        return None;
    }
    // 4: `[0-9_]+[eE](?:[0-9_])?+[FfDdMm]?`, 5: `[0-9_]+\.[eE](?:[0-9_])?+[FfDdMm]?`
    for at in [i, i + 1] {
        if at == i + 1 && b.get(i) != Some(&b'.') {
            continue;
        }
        if matches!(b.get(at), Some(b'e' | b'E')) {
            let mut e = at + 1;
            if b.get(e).is_some_and(|&c| c.is_ascii_digit() || c == b'_') {
                e += 1;
            }
            if tf(e)
                && let Some(r) = tail(b, e + 1)
            {
                return Some((r.0, r.1, INTEGER));
            }
            if let Some(r) = tail(b, e) {
                return Some((r.0, r.1, INTEGER));
            }
        }
    }
    // 6: `[0-9_]+[\.]?[FfDdMm]`
    for at in [i + 1, i] {
        if at == i + 1 && b.get(i) != Some(&b'.') {
            continue;
        }
        if tf(at)
            && let Some(r) = tail(b, at + 1)
        {
            return Some((r.0, r.1, INTEGER));
        }
    }
    // 7: `[0-9_]+[\.]?(U|L|UL|…)?`
    for at in [i + 1, i] {
        if at == i + 1 && b.get(i) != Some(&b'.') {
            continue;
        }
        if let Some(r) = suffixed_tail(b, at) {
            return Some((r.0, r.1, INTEGER));
        }
    }
    None
}

/// `(?:U|u|L|l|UL|…)?((?i:[kmgtp]b)?)\b` at `at`: where it ends and the unit starts.
fn suffixed_tail(b: &[u8], at: usize) -> Option<(usize, usize)> {
    let ul = |c: u8| matches!(c, b'U' | b'u' | b'L' | b'l');
    if let Some(&c) = b.get(at)
        && ul(c)
    {
        if let Some(r) = tail(b, at + 1) {
            return Some(r);
        }
        if let Some(&d) = b.get(at + 1)
            && ul(d)
            && (c.eq_ignore_ascii_case(&b'u') != d.eq_ignore_ascii_case(&b'u'))
            && let Some(r) = tail(b, at + 2)
        {
            return Some(r);
        }
    }
    tail(b, at)
}

/// `((?i:[kmgtp]b)?)\b` at `at`: where it ends and the unit starts.
fn tail(b: &[u8], at: usize) -> Option<(usize, usize)> {
    if b.get(at).is_some_and(|c| b"kmgtpKMGTP".contains(c))
        && matches!(b.get(at + 1), Some(b'b' | b'B'))
        && boundary(b, at + 2)
    {
        return Some((at + 2, at));
    }
    boundary(b, at).then_some((at, at))
}

/// An escape at `p`, `` ` ``: `` `[`0abefnrtv'"‘-„$] ``, `` `u{…} ``, or the invalid
/// `` `u(?:\{[0-9a-fA-F]{,6}.)? ``: where it ends and its leaf.
fn escape(b: &[u8], p: usize) -> Option<(usize, u16)> {
    let c = *b.get(p + 1)?;
    match c {
        b'`' | b'0' | b'a' | b'b' | b'e' | b'f' | b'n' | b'r' | b't' | b'v' | b'\'' | b'"'
        | b'$' => Some((p + 2, ESCAPE)),
        0xe2 if b.get(p + 2) == Some(&0x80) && matches!(b.get(p + 3), Some(0x98..=0x9e)) => {
            Some((p + 4, ESCAPE))
        }
        b'u' => {
            let o = p + 2;
            if b.get(o) == Some(&b'{') {
                let mut h = o + 1;
                while h < b.len() && b[h].is_ascii_hexdigit() {
                    h += 1;
                }
                let l = h - (o + 1);
                if b.get(h) == Some(&b'}') {
                    let d = &b[o + 1..h];
                    let ok =
                        (1..=5).contains(&l) || (l == 6 && (d.starts_with(b"10") || d[0] == b'0'));
                    if ok {
                        return Some((h + 1, ESCAPE));
                    }
                }
                // `\{[0-9a-fA-F]{,6}.`: `.` one character, not the line's end
                let h6 = h.min(o + 1 + 6);
                if h6 < b.len() {
                    return Some((h6 + char_len(b[h6]), INVALID_ESCAPE));
                }
                if h6 > o + 1 {
                    // the last digit is `.`'s
                    return Some((h6, INVALID_ESCAPE));
                }
            }
            Some((o, INVALID_ESCAPE))
        }
        _ => None,
    }
}

/// The bytes a string's patterns start on, around a regex: `'…'`'s, then `"…"`'s.
static RE_YIELDS: [[bool; 256]; 2] = [
    crate::regex::table(b"'\xe2"),
    crate::regex::table(b"\"$`\xe2"),
];

/// A regex in a `'…'` string, then in a `"…"` one; `base` and `text` set where it is used.
static RE_EMBEDS: [crate::regex::Embed<'static>; 2] = [
    crate::regex::Embed {
        dialect: crate::regex::Dialect::DotNet,
        cooked: false,
        yields: &RE_YIELDS[0],
        stop: |b, p, _| single_quote(b, p).is_some(),
        arg: 0,
        escape: |_, p| p + 1,
        top_only: false,
        continues: false,
        base: 0,
        text: 0,
    },
    crate::regex::Embed {
        dialect: crate::regex::Dialect::DotNet,
        cooked: false,
        yields: &RE_YIELDS[1],
        stop: |b, p, _| match b[p] {
            b'`' => true,
            // a variable's `$`, or `$(`
            b'$' => b.get(p + 1).is_some_and(|&c| {
                WORD[usize::from(c)] || c >= 0x80 || matches!(c, b'$' | b'^' | b'?' | b'{' | b'(')
            }),
            _ => double_quote(b, p).is_some(),
        },
        arg: 0,
        escape: |_, p| p + 1,
        top_only: false,
        continues: false,
        base: 0,
        text: 0,
    },
];

/// Where the blanks before `p` start.
fn blank_back(b: &[u8], p: usize) -> usize {
    let mut q = p;
    while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
        q -= 1;
    }
    q
}

/// Whether `b[..e]` ends with `name`, any case, with no word character before it.
fn ends_word_ci(b: &[u8], e: usize, name: &[u8]) -> bool {
    e >= name.len()
        && b[e - name.len()..e].eq_ignore_ascii_case(name)
        && (e == name.len() || !WORD[usize::from(b[e - name.len() - 1])])
}

/// The static methods of `[regex]` that take the regex second, after the input.
const REGEX_METHODS: [&[u8]; 5] = [b"ismatch", b"match", b"matches", b"replace", b"split"];

/// Whether a string opening after `b[..q]` (its blanks before it passed), on a line with the
/// type `[regex]`, is a regex: after `[regex]` (or `[System.Text.RegularExpressions.Regex]`), a
/// cast; the first argument of `[regex]::new(`; the second of `[regex]::Match(` and the like.
#[cold]
#[inline(never)]
fn regex_typed(b: &[u8], q: usize) -> bool {
    match b[q - 1] {
        b']' => ends_word_ci(b, q, b"[regex]") || ends_word_ci(b, q, b".regex]"),
        b'(' => {
            let w = blank_back(b, q - 1);
            ends_word_ci(b, w, b"regex]::new")
        }
        _ => {
            // the call's `(`, over its first argument (up to 512 bytes back)
            let mut depth = 0usize;
            let mut i = q - 1;
            let stop = i.saturating_sub(512);
            loop {
                if i <= stop {
                    return false;
                }
                i -= 1;
                match b[i] {
                    b')' | b']' | b'}' => depth += 1,
                    b'(' if depth == 0 => break,
                    b'(' | b'[' | b'{' => match depth.checked_sub(1) {
                        Some(d) => depth = d,
                        None => return false,
                    },
                    b',' if depth == 0 => return false,
                    _ => {}
                }
            }
            let w = blank_back(b, i);
            let mut s = w;
            while s > 0 && b[s - 1].is_ascii_alphabetic() {
                s -= 1;
            }
            REGEX_METHODS
                .iter()
                .any(|m| b[s..w].eq_ignore_ascii_case(m))
                && s >= 9
                && b[s - 9..s].eq_ignore_ascii_case(b"[regex]::")
        }
    }
}

/// A single quote, `['‘-‛]`, at `p`: its length.
fn single_quote(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        b'\'' => Some(1),
        0xe2 if b.get(p + 1) == Some(&0x80) && matches!(b.get(p + 2), Some(0x98..=0x9b)) => Some(3),
        _ => None,
    }
}

/// A double quote, `["“-„]`, at `p`: its length.
fn double_quote(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        b'"' => Some(1),
        0xe2 if b.get(p + 1) == Some(&0x80) && matches!(b.get(p + 2), Some(0x9c..=0x9e)) => Some(3),
        _ => None,
    }
}

/// Where the run of a command's prefix bytes, `[\p{L}\d_\-\\:]`, at `p` ends.
fn prefix_run(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        let c = b[p];
        if WORD[usize::from(c)] || matches!(c, b'-' | b'\\' | b':') {
            p += 1;
        } else if c >= 0x80 && (is_letter(b, p) || digit(b, p).is_some()) {
            p += char_len(c);
        } else {
            break;
        }
    }
    p
}

/// Where the ASCII letters at `p` end.
fn alpha_end(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && b[p].is_ascii_alphabetic() {
        p += 1;
    }
    p
}

/// Where a function's name, `(?:\p{L}|\d|_|-|\.)+`, at `p` ends.
fn fn_word_end(b: &[u8], mut p: usize) -> usize {
    loop {
        match b.get(p) {
            Some(&c) if WORD[usize::from(c)] || c == b'-' || c == b'.' => p += 1,
            Some(&c) if c >= 0x80 && is_word(b, p) => p += char_len(c),
            _ => return p,
        }
    }
}

/// `\b` at `p`: a word's character on one side only (the line's `\n` past its end).
fn boundary(b: &[u8], p: usize) -> bool {
    let after = is_word(b, p);
    let before = if p == 0 {
        false
    } else {
        let mut s = p - 1;
        while s > 0 && b[s] & 0xc0 == 0x80 {
            s -= 1;
        }
        is_word(b, s)
    };
    before != after
}

/// The bytes of the UTF-8 character that starts with `c`.
fn char_len(c: u8) -> usize {
    match c {
        0xf0.. => 4,
        0xe0.. => 3,
        0xc0.. => 2,
        _ => 1,
    }
}

/// The character that starts at byte `i` of `b`, past ASCII.
#[cold]
fn wide_char(b: &[u8], i: usize) -> Option<char> {
    let e = (i + char_len(b[i])).min(b.len());
    std::str::from_utf8(&b[i..e]).ok()?.chars().next()
}

/// Whether `c` is one of Unicode's decimal digits, `\p{Nd}`.
fn is_nd(c: char) -> bool {
    let u = c as u32;
    c.is_ascii_digit() || crate::csharp::ND.iter().any(|&s| u >= s && u - s < 10)
}

/// Whether the character at byte `i` of `b` is a word's, `\w`, or a name's, `(?:\p{L}|\d|_)`:
/// past ASCII, a letter or a decimal digit. (Oniguruma's Latin-1 table also marks `²³¹¼½¾`
/// as words, though its Unicode table leaves `\p{No}` out; not followed.) `i` is a
/// character's first byte.
fn is_word(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => WORD[usize::from(c)],
        Some(_) => wide_char(b, i).is_some_and(|c| c.is_alphabetic() || is_nd(c)),
        None => false,
    }
}

/// Whether the character at byte `i` is a letter, `\p{L}`.
fn is_letter(b: &[u8], i: usize) -> bool {
    match b.get(i) {
        Some(&c) if c < 0x80 => c.is_ascii_alphabetic(),
        Some(_) => wide_char(b, i).is_some_and(char::is_alphabetic),
        None => false,
    }
}

/// Where the word at `p` ends.
fn word_end(b: &[u8], mut p: usize) -> usize {
    loop {
        while p < b.len() && WORD[usize::from(b[p])] {
            p += 1;
        }
        if p < b.len() && b[p] >= 0x80 && is_word(b, p) {
            p += char_len(b[p]);
        } else {
            return p.min(b.len());
        }
    }
}

/// `\s` at `p` of the line (not its `\n`): how many bytes the space takes.
fn space_len(b: &[u8], p: usize) -> Option<usize> {
    let &c = b.get(p)?;
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r').then_some(1);
    }
    wide_space(b, p)
}

#[cold]
fn wide_space(b: &[u8], p: usize) -> Option<usize> {
    let c = wide_char(b, p)?;
    matches!(
        c,
        '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
    .then(|| c.len_utf8())
}

/// `\d` at `p`, Unicode's decimal digits: where it ends.
fn digit(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        c if c.is_ascii_digit() => Some(p + 1),
        c if c < 0x80 => None,
        _ => wide_char(b, p)
            .filter(|&c| is_nd(c))
            .map(|c| p + c.len_utf8()),
    }
}

/// `\d*` at `p`: where it ends.
fn digits0(b: &[u8], mut p: usize) -> usize {
    while let Some(e) = digit(b, p) {
        p = e;
    }
    p
}

/// Whether `w` starts the bytes at `p`, ignoring ASCII case.
fn at_ci(b: &[u8], p: usize, w: &[u8]) -> bool {
    b.len() >= p + w.len() && b[p..p + w.len()].eq_ignore_ascii_case(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lang;
    use crate::tests::marked;

    fn kinds(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::PowerShell, lines)
    }

    #[test]
    fn the_table_finds_its_words() {
        for &(w, k) in ENTRIES {
            assert_eq!(lookup(w), k, "{}", String::from_utf8_lossy(w));
            assert_eq!(lookup(&w.to_ascii_uppercase()), k);
        }
        assert_eq!(lookup(b"gets"), 0);
        assert_eq!(lookup(b"valuefrompipelinebypropertynames"), 0);
    }

    /// The patterns that search or backtrack (`${` with no `}`, a command's `…\` prefix, a
    /// run of `a-a-…` before `.exe`, a verb's `.+?` to the line's end, a number's digits) are
    /// memoized or bounded, so a long line stays linear.
    #[test]
    fn long_lines_stay_linear() {
        let lines = [
            "${".repeat(30000),
            format!("{}\\x", "a-".repeat(30000)),
            format!("{}.", "a-".repeat(30000)),
            format!("Get-{}", "-".repeat(60000)),
            format!("{}x", "1".repeat(60000)),
            "$x.".repeat(20000),
            "`u{".repeat(20000),
            "[regex]::Match((a), 'x', ".repeat(5000),
            "$a -match '(?<n>(x)' ".repeat(5000),
        ];
        let t = std::time::Instant::now();
        let (mut state, mut spans) = (State::default(), Vec::new());
        for line in &lines {
            crate::lex(Lang::PowerShell, line, &mut state, &mut spans);
        }
        assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
    }

    #[test]
    fn commands_variables_and_operators() {
        assert_eq!(
            kinds(&["Get-Item C:\\x\\Set-Y.ps1 | % { $_.Name -eq $env:A } 2>&1"]),
            [
                ("Get-Item", "support.function.powershell"),
                ("C:\\x\\Set-Y.ps1", "support.function.powershell"),
                ("|", "keyword.operator.other.powershell"),
                ("%", "keyword.control.powershell"),
                ("{", "punctuation.section.braces.begin.powershell"),
                (
                    "$",
                    "support.variable.automatic.powershell punctuation.definition.variable.powershell"
                ),
                ("_", "support.variable.automatic.powershell"),
                (
                    ".Name",
                    "support.variable.automatic.powershell variable.other.member.powershell"
                ),
                ("-eq", "keyword.operator.comparison.powershell"),
                (
                    "$",
                    "variable.other.readwrite.powershell punctuation.definition.variable.powershell"
                ),
                (
                    "env:",
                    "variable.other.readwrite.powershell support.variable.drive.powershell"
                ),
                ("A", "variable.other.readwrite.powershell"),
                ("}", "punctuation.section.braces.end.powershell"),
                ("2>&1", "keyword.operator.redirection.powershell"),
            ]
        );
    }

    /// Here-strings and block comments carry their state between lines; a `#` comment and
    /// `#requires` end with theirs.
    #[test]
    fn regexes_in_strings() {
        const S: &str = "string.quoted.single.powershell";
        const D: &str = "string.quoted.double.powershell";
        let m = kinds(&[
            r#"if ($a -match '^(?<y>\d{4})''s$') { $b -CReplace "\s+$x`t(?#c)$", '$1' }"#,
            r#"$p = [regex]'[a-z-[aeiou]]'; [regex]::Matches($s, "(\w)\1"); -split 'a b'"#,
            r#"$s -split '\s*,\s*' | Select-String -Pattern "^$($n)(\d"#,
            r#"x+)" -NotMatch; $t -like '^a*'"#,
        ]);
        let has = |text: &str, scopes: &str| {
            assert!(
                m.iter().any(|&(t, s)| t == text && s == scopes),
                "{text:?} as {scopes:?} in {m:#?}"
            );
        };
        has("^", &format!("{S} keyword.control.anchor.regexp"));
        has("y", &format!("{S} entity.name.other.group.regexp"));
        has(
            r"\d",
            &format!("{S} constant.other.character-class.escape.backslash.regexp"),
        );
        // the host's `''`, then the regex's `$`
        has("''", &format!("{S} constant.character.escape.powershell"));
        has("$", &format!("{S} keyword.control.anchor.regexp"));
        // `"…"`: a variable and an escape are the host's, a `$` before `"` the regex's
        has("x", &format!("{D} variable.other.readwrite.powershell"));
        has("`t", &format!("{D} constant.character.escape.powershell"));
        has(
            "(?#",
            &format!("{D} comment.block.regexp punctuation.definition.comment.begin.regexp"),
        );
        has("$", &format!("{D} keyword.control.anchor.regexp"));
        // a cast, a subtraction; `[regex]::Matches(`'s second argument
        has(
            "-",
            &format!(
                "{S} constant.other.character-class.set.regexp keyword.operator.difference.regexp"
            ),
        );
        has(r"\1", &format!("{D} keyword.other.back-reference.regexp"));
        // a binary `-split`, `-Pattern`, a regex across `$( )` and lines
        has("*", &format!("{S} keyword.operator.quantifier.regexp"));
        has(
            "(",
            &format!("{D} punctuation.definition.group.begin.regexp"),
        );
        has("+", &format!("{D} keyword.operator.quantifier.regexp"));
        has(")", &format!("{D} punctuation.definition.group.end.regexp"));
        // not regexes: `-replace`'s second operand, a unary `-split`, `-like`
        for t in ["$1", "a b", "^a*"] {
            assert!(
                m.iter().any(|&(x, s)| x == t && (s == S || s == D)),
                "{t:?} in {m:#?}"
            );
        }
    }

    #[test]
    fn state_between_lines() {
        assert_eq!(
            kinds(&["$s = @\"", "a $(b)", "\"@ # c", "<# .NOTES", "#>"]),
            [
                (
                    "$",
                    "variable.other.readwrite.powershell punctuation.definition.variable.powershell"
                ),
                ("s", "variable.other.readwrite.powershell"),
                ("=", "keyword.operator.assignment.powershell"),
                (
                    "@\"",
                    "string.quoted.double.heredoc.powershell punctuation.definition.string.begin.powershell"
                ),
                ("a ", "string.quoted.double.heredoc.powershell"),
                (
                    "$",
                    "string.quoted.double.heredoc.powershell keyword.other.substatement.powershell \
                     punctuation.definition.substatement.powershell \
                     punctuation.section.embedded.substatement.begin.powershell"
                ),
                (
                    "(",
                    "string.quoted.double.heredoc.powershell punctuation.section.group.begin.powershell \
                     punctuation.section.embedded.substatement.begin.powershell"
                ),
                ("b", "string.quoted.double.heredoc.powershell"),
                (
                    ")",
                    "string.quoted.double.heredoc.powershell punctuation.section.group.end.powershell \
                     punctuation.section.embedded.substatement.end.powershell"
                ),
                (
                    "\"@",
                    "string.quoted.double.heredoc.powershell punctuation.definition.string.end.powershell"
                ),
                (
                    "#",
                    "comment.line.powershell punctuation.definition.comment.powershell"
                ),
                (" c", "comment.line.powershell"),
                (
                    "<#",
                    "comment.block.powershell punctuation.definition.comment.block.begin.powershell"
                ),
                (
                    " ",
                    "comment.block.powershell comment.documentation.embedded.powershell"
                ),
                (
                    ".",
                    "comment.block.powershell comment.documentation.embedded.powershell \
                     constant.string.documentation.powershell"
                ),
                (
                    "NOTES",
                    "comment.block.powershell comment.documentation.embedded.powershell \
                     keyword.operator.documentation.powershell"
                ),
                (
                    "#>",
                    "comment.block.powershell punctuation.definition.comment.block.end.powershell"
                ),
            ]
        );
        let mut state = State::default();
        let mut spans = Vec::new();
        crate::lex(Lang::PowerShell, "#requires", &mut state, &mut spans);
        assert_eq!(state, State::default());
    }
}
