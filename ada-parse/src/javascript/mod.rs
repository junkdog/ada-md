//! JavaScript by a scanner of ada's own (ADR-40, speed first): bat's `JavaScript (Babel)`
//! grammar compiled by hand, as Go's and Python's were. Each of its contexts that can be open is
//! an arm of one `match` on the top of a stack, trying its patterns in the grammar's order at
//! the place the scan has reached, which is syntect's leftmost match; where no pattern can
//! start, the scan steps on.
//!
//! The grammar (123 contexts, no prototype) is the old regex style: lookbehinds on the line,
//! lookaheads over a balanced `(…)` (`\g<-1>`), and back-references for a string's quote and a
//! JSX element's closing tag. **syntect's guard against a loop** is followed: a pop that matches
//! nothing right after a push that matched nothing there, at the depth that push left, is passed
//! over for the context's later patterns, else the scan steps a character. Contexts that differ
//! only in a meta scope no theme can see are one here (the four class methods' heads, the
//! functions', the arrows'); the anonymous ones are named by what they read.
//!
//! A regex literal's body is [`crate::regex`]'s ECMAScript dialect (bat's `Regular Expressions
//! (Javascript)`, which Babel's grammar includes), its state in the literal's context byte; its
//! flags are ECMAScript's, `[dgimsuvy]`, where bat's grammar takes `[gimy]`. **Not followed, by
//! design:** GraphQL in a ``gql`…` `` template is plain text (bat embeds its GraphQL grammar). A kind is a leaf, bat's scopes for a token ([`scopes`]); of
//! the meta scopes only those a theme can see are kept (a string's, a regex's, a comment's).
//!
//! The stack between lines is the contexts open above the top level, up to 28 (the bytes of
//! [`State`]'s levels). A JSX element's context keeps its tag's name as a 7-bit hash there, so
//! a closing tag on a later line is matched by its hash; on its own line, by its text.

mod code;

use crate::emit::{Push, Rules, Run, Runs, Sink, Spans};
use crate::prim::{WORD, char_len, digit, is_word, stops, wide_digit, wide_space, word_end};
use crate::regex;
use crate::{Out, State};

// The contexts, by bat's names where they have one.
/// `main`: `core`.
const MAIN: u8 = 0;
/// `curly-brackets`' `{ … }`: `main`.
const CURLY: u8 = 1;
/// `round-brackets`' `( … )`: `expression`.
const ROUND: u8 = 2;
/// `square-brackets`' `[ … ]`: `expression`.
const SQUARE: u8 = 3;
/// A `/** … */` comment.
const DOC: u8 = 4;
/// A `/* … */` comment.
const BLOCK: u8 = 5;
/// A `"` string.
const STR_D: u8 = 6;
/// A `'` string.
const STR_S: u8 = 7;
/// A template string's text.
const TPL: u8 = 8;
/// A template's `${ … }`: `expression`.
const SUBST: u8 = 9;
/// A regex literal's body: this, or'd with the regex's state ([`regex::ECMA_STATE`], its
/// groups open and whether a class is), up to [`RE_LAST`].
const REGEX: u8 = 0x60;
const RE_LAST: u8 = REGEX | regex::ECMA_STATE;
const _: () = assert!(REGEX & regex::ECMA_STATE == 0 && RE_LAST < JSX_TAG);
/// A regex string, above its string's or template's context: `RegExp("…")`'s first argument.
/// Or'd with the regex's state, as [`REGEX`], up to [`RE_STR_LAST`].
const RE_STR: u8 = 0x40;
const RE_STR_LAST: u8 = RE_STR | regex::ECMA_STATE;
const _: () = assert!(RE_STR & regex::ECMA_STATE == 0 && RE_STR_LAST < REGEX);
/// After `for`, up to its `( … )`.
const FOR: u8 = 13;
/// Inside a `for`'s `( … )`.
const FOR_IN: u8 = 14;
/// After `switch`, up to its `{ … }`.
const SWITCH: u8 = 15;
/// A `switch`'s `{ … }`.
const SWITCH_BODY: u8 = 16;
/// After `case` or `default`, up to its `:`.
const CASE: u8 = 17;
/// After `class` or `interface`, up to its `}`.
const CLASS: u8 = 18;
/// After the class's name, up to its `{`.
const CLASS_HEAD: u8 = 19;
/// After `extends`, up to the class's `{`.
const CLASS_EXTENDS: u8 = 20;
/// The class's `{ … }`.
const CLASS_BODY: u8 = 21;
/// A class property, `name = …` or `name: …`, up to `;` or `}`.
const CLASS_PROP: u8 = 22;
/// A class method's head, up to its `{` or `;` (`class-method-definition`'s four).
const METHOD_DEF: u8 = 23;
/// A function's head, up to its `)` (`literal-function`'s three, `literal-function-labels`).
const FUNC: u8 = 24;
/// An object method's head, up to its `)` (`literal-method`'s two).
const METHOD: u8 = 25;
/// An arrow function's parameters, up to `) =>` (`literal-arrow-function`'s three,
/// `literal-arrow-function-labels`).
const ARROW: u8 = 26;
/// A parameter list, `( … )`.
const PARAMS: u8 = 27;
/// A parameter's default value, up to `,` or `)`.
const PARAM_DEFAULT: u8 = 28;
/// After `new`, its constructor's name.
const CTOR: u8 = 29;
/// A quoted object key, up to its `:`.
const LABEL: u8 = 30;
/// A JSX element's place, from before its `<` to after its last `>`.
const JSX: u8 = 31;
/// A JSX element's children, after its opening tag.
const JSX_CHILDREN: u8 = 32;
/// A JSX `{ … }`.
const JSX_EXPR: u8 = 33;
/// A JSX attribute's `"` string.
const JSTR_D: u8 = 34;
/// A JSX attribute's `'` string.
const JSTR_S: u8 = 35;
/// A Flow annotation after `:`.
const FLOW_ANNOT: u8 = 36;
/// Flow's `{ … }`.
const FLOW_BRACES: u8 = 37;
/// After `type Name`, up to `;` or `from`.
const FLOW_DECL: u8 = 38;
/// After `type {`, up to after its `}`.
const FLOW_DECL_BRACES: u8 = 39;
/// Flow's `< … >` (`flowtype-polymorph`, and `flowtype-tokens`' own).
const FLOW_ANGLE: u8 = 40;
/// After a Flow `:`, `?`, `|`, `&` or `=` before `{`, up to after its `}`.
const FLOW_AFTER: u8 = 41;
/// Flow's `[ … ]`.
const FLOW_SQUARE: u8 = 42;
/// Flow's `( … )`.
const FLOW_ROUND: u8 = 43;
/// After Flow's `=>`, its return type.
const FLOW_FN: u8 = 44;
/// A ``gql`…` `` template: plain text (bat embeds GraphQL; not followed).
const GRAPHQL: u8 = 45;
/// A GraphQL template's `${ … }`: `core`.
const GQL_SUBST: u8 = 46;
/// A styled-components template: plain text up to its `` ` `` (its CSS grammar,
/// `source.js.css`, isn't in bat's set).
const STYLED: u8 = 47;
/// `styled(…)` before its template.
const STYLED_CALL: u8 = 48;
/// `styled(…)`'s arguments.
const STYLED_ARGS: u8 = 49;
/// A `{{ … }}`.
const MUSTACHE: u8 = 50;
/// A JSX element's opening tag, its attributes, and where it closes: this bit and a 7-bit hash
/// of its tag's name.
const JSX_TAG: u8 = 0x80;

macro_rules! leaves {
    ($($name:ident = $scopes:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* }
        $(#[allow(dead_code)] const $name: u16 = Leaf::$name as u16;)*
        /// Each kind's scopes under `source.js`, outermost first.
        static LEAVES: &[&str] = &[$($scopes,)*];
    };
}

leaves! {
    PLAIN = "",
    DOC_TEXT = "comment.block.documentation.js",
    DOC_MARK = "comment.block.documentation.js punctuation.definition.comment.js",
    BLOCK_TEXT = "comment.block.js",
    BLOCK_BEGIN = "comment.block.js punctuation.definition.comment.begin.js",
    BLOCK_END = "comment.block.js punctuation.definition.comment.end.js",
    HTML_COMMENT = "comment.block.html.js punctuation.definition.comment.js",
    LINE = "comment.line.double-slash.js",
    LINE_MARK = "comment.line.double-slash.js punctuation.definition.comment.js",
    SHEBANG = "comment.line.shebang.js",
    SHEBANG_MARK = "comment.line.shebang.js punctuation.definition.comment.js",
    STR = "string.quoted.js",
    STR_BEGIN = "string.quoted.js punctuation.definition.string.begin.js",
    STR_END = "string.quoted.js punctuation.definition.string.end.js",
    STR_ESC = "string.quoted.js constant.character.escape.js",
    STR_ESC_NL = "string.quoted.js constant.character.escape.newline.js",
    KEY_FN = "string.quoted.js entity.name.function.js",
    KEY_NAME_FN = "string.unquoted.js entity.name.function.js",
    TPL_BEGIN = "string.interpolated.js keyword.other.template.begin.js",
    TPL_END = "string.interpolated.js keyword.other.template.end.js",
    TPL_TEXT = "string.interpolated.js",
    ESC = "constant.character.escape.js",
    ESC_NL = "constant.character.escape.newline.js",
    SUBST_BEGIN = "keyword.other.substitution.begin.js",
    SUBST_END = "keyword.other.substitution.end.js",
    RE = "string.regexp.js",
    RE_BEGIN = "string.regexp.js punctuation.definition.string.begin.js",
    RE_END = "string.regexp.js punctuation.definition.string.end.js",
    RE_FLAGS = "string.regexp.js keyword.other.js",
    OBJ_KEY = "constant.other.object.key.js",
    OBJ_KEY_LABEL = "constant.other.object.key.js string.unquoted.label.js",
    OBJ_KEY_SEP = "constant.other.object.key.js punctuation.separator.key-value.js",
    KEY_SEP = "punctuation.separator.key-value.js",
    ENTITY_FN = "entity.name.function.js",
    ASSIGN = "keyword.operator.assignment.js",
    STORAGE = "storage.type.js",
    STORAGE_FN = "storage.type.function.js",
    ASTERISK = "keyword.generator.asterisk.js",
    ENTITY_CLASS = "entity.name.class.js",
    ACCESSOR = "keyword.operator.accessor.js",
    PROTOTYPE = "variable.language.prototype.js",
    ARROW_KW = "storage.type.function.arrow.js",
    PARAM = "variable.parameter.function.js",
    LOOP = "keyword.control.loop.js",
    SWITCH_KW = "keyword.control.switch.js",
    FLOW_KW = "keyword.control.flow.js",
    COND = "keyword.control.conditional.js",
    TRY = "keyword.control.trycatch.js",
    RESERVED = "keyword.other.reserved.js",
    KW_OTHER = "keyword.other.js",
    MODULE = "keyword.operator.module.js",
    CLASS_KW = "storage.type.class.js",
    INTERFACE_KW = "storage.type.interface.flowtype.js",
    EXTENDS = "storage.type.extends.js",
    DECLARE = "support.type.declare.flowtype.js",
    TYPE_KW = "support.type.type.flowtype.js",
    ENTITY_TYPE = "entity.name.type.js",
    INVALID_HEX = "invalid.illegal.numeric.hex.js",
    INVALID_OCT = "invalid.illegal.numeric.octal.js",
    NUMBER = "constant.numeric.js",
    INFINITY = "constant.language.infinity.js",
    TRUE = "constant.language.boolean.true.js",
    FALSE = "constant.language.boolean.false.js",
    NULL = "constant.language.null.js",
    UNDEFINED = "constant.language.undefined.js",
    NAN = "constant.language.nan.js",
    ARGUMENTS = "variable.language.arguments.js",
    SUPER = "variable.language.super.js",
    THIS = "variable.language.this.js",
    SELF = "variable.language.self.js",
    PROTO = "variable.language.proto.js",
    CONSTRUCTOR = "variable.language.constructor.js",
    NEW = "keyword.operator.new.js",
    CTOR_NAME = "variable.function.constructor.js",
    CLASS_VAR = "variable.other.class.js",
    FUNCTION = "variable.function.js",
    CALL_PARENS = "meta.group.braces.round.function.arguments.js",
    ROUND_BEGIN = "meta.brace.round.begin.js",
    ROUND_END = "meta.brace.round.end.js",
    SQUARE_BEGIN = "meta.brace.square.begin.js",
    SQUARE_END = "meta.brace.square.end.js",
    CURLY_BEGIN = "meta.brace.curly.begin.js",
    CURLY_END = "meta.brace.curly.end.js",
    OP = "keyword.operator.js",
    LOGICAL = "keyword.operator.logical.js",
    AUGMENTED = "keyword.operator.assignment.augmented.js",
    BITWISE = "keyword.operator.bitwise.js",
    RELATIONAL = "keyword.operator.relational.js",
    COMPARISON = "keyword.operator.comparison.js",
    ARITHMETIC = "keyword.operator.arithmetic.js",
    TERNARY = "keyword.operator.ternary.js",
    SPREAD = "keyword.operator.spread.js",
    CONSTANT = "variable.other.constant.js",
    STATIC_PROP = "variable.other.property.static.js",
    OBJECT = "variable.other.object.js",
    PROPERTY = "variable.other.property.js",
    READWRITE = "variable.other.readwrite.js",
    TERMINATOR = "punctuation.terminator.statement.js",
    COMMA = "meta.delimiter.comma.js",
    DECORATOR = "tag.decorator.js",
    DEC_MARK = "tag.decorator.js punctuation.definition.tag.js",
    DEC_NAME = "tag.decorator.js entity.name.tag.js",
    BUILTIN_CLASS = "support.class.builtin.js",
    ERROR_CLASS = "support.class.error.js",
    NODE_CLASS = "support.class.node.js",
    BUILTIN_FN = "support.function.builtin.js",
    NODE_FN = "support.function.node.js",
    DOM = "support.type.object.dom.js",
    NODE_OBJ = "support.type.object.node.js",
    CONSOLE = "support.type.object.console.js",
    CONSOLE_FN = "support.function.console.js",
    PROCESS = "support.type.object.process.js",
    PROCESS_FN = "support.function.process.js",
    MODULE_OBJ = "support.type.object.module.js",
    CONFLICT_INTO = "invalid.illegal.conflict-marker.merge-into.js",
    CONFLICT_SEP = "invalid.illegal.conflict-marker.separator.js",
    CONFLICT_OTHER = "invalid.illegal.conflict-marker.other-commit.js",
    CONFLICT_TEXT = "invalid.illegal.string.js",
    MODIFIER = "storage.modifier.js",
    ACCESSOR_KW = "storage.type.accessor.js",
    ENTITY_ACCESSOR = "entity.name.accessor.js",
    FLOW_OP = "keyword.operator.flowtype.js",
    PARAMS_BEGIN = "punctuation.definition.parameters.begin.js",
    PARAMS_END = "punctuation.definition.parameters.end.js",
    PARAM_SEP = "punctuation.separator.parameter.function.js",
    FLOW_OPTIONAL = "keyword.operator.flowtype.optional.js",
    FLOW_ANNOTATION = "keyword.operator.flowtype.annotation.js",
    FLOW_OTHER = "keyword.operator.flowtype.other.js",
    FLOW_BEGIN = "punctuation.section.flowtype.begin.js",
    FLOW_END = "punctuation.section.flowtype.end.js",
    FLOW_PRIMITIVE = "constant.other.primitve.flowtype.js",
    FLOW_VAR = "variable.other.flowtype.js",
    TAG_BEGIN = "meta.tag.jsx punctuation.definition.tag.begin.jsx",
    TAG_NAME = "meta.tag.jsx entity.name.tag.jsx",
    TAG_END = "meta.tag.jsx punctuation.definition.tag.end.jsx",
    TAG_INCOMPLETE = "invalid.illegal.tag.incomplete.jsx",
    ATTR = "entity.other.attribute-name.jsx",
    ATTR_EQ = "keyword.operator.assignment.jsx",
    JSTR = "string.quoted.jsx",
    JSTR_BEGIN = "string.quoted.jsx punctuation.definition.string.begin.jsx",
    JSTR_END = "string.quoted.jsx punctuation.definition.string.end.jsx",
    JSTR_ENTITY = "string.quoted.jsx constant.character.entity.jsx",
    JSTR_BAD = "string.quoted.jsx invalid.illegal.bad-ampersand.jsx",
    ENTITY = "constant.character.entity.jsx",
    BAD_AMP = "invalid.illegal.bad-ampersand.jsx",
    EMBED_BEGIN = "punctuation.section.embedded.begin.jsx",
    EMBED_END = "punctuation.section.embedded.end.jsx",
    TAGGED = "variable.function.tagged-template.js",
    TPL_MARK_BEGIN = "punctuation.definition.string.template.begin.js",
    TPL_MARK_END = "punctuation.definition.string.template.end.js",
    PUNCT_ACCESSOR = "punctuation.accessor.js",
    STYLED_FN = "meta.function-call.with-arguments.js variable.function.js",
    GROUP_BEGIN = "punctuation.definition.group.begin.js",
    GROUP_END = "punctuation.definition.group.end.js",
}

/// The kinds of a regex's tokens: the regex's, from here, [`regex::KINDS`] for each host: a
/// literal, a quoted string, a template.
const RE_KINDS: u16 = 256;
const _: () = assert!(LEAVES.len() <= RE_KINDS as usize);

/// The scopes under `source.js` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    if kind >= RE_KINDS {
        let n = regex::KINDS as u16;
        let host = match (kind - RE_KINDS) / n {
            0 => "string.regexp.js",
            1 => "string.quoted.js",
            2 => "string.interpolated.js",
            _ => return Vec::new(),
        };
        let mut out = vec![host];
        out.extend(regex::scopes((kind - RE_KINDS) % n));
        return out;
    }
    LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
        l.split(' ').filter(|s| !s.is_empty()).collect()
    })
}

/// How many contexts a line can hold; one pushed past it drops the bottom one.
const MAX: usize = 128;
/// How many of the JSX tags pushed on a line keep their place in it; an older one is matched
/// by its hash, as one from an earlier line.
const TAGS: usize = 16;
/// Matches that take nothing, in a row at one place, before the scan steps on regardless.
const STUCK: usize = 64;
/// syntect's stack limit (with its two bottom contexts, `__start` and `__main`): at it, a
/// push's pattern matches nothing, so the patterns after it get their turn.
const SYNTECT_DEPTH: usize = 99;

/// Lex `b`, a line of JavaScript without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, 0, state, out, Spans::new());
    s.load_state(state);
    s.k.open(s.out, b.len() / 4 + 16);
    s.lex();
    s.k.close(s.out);
    *state = State::keep(&s.stack, s.depth, s.exact, 0);
}

/// Lex `b`, a line holding JavaScript from `from` (HTML's `<script>` and `on…="…"`), from `state`.
#[inline(never)]
pub(crate) fn line_from(b: &[u8], from: usize, state: &mut State, out: &mut Out) {
    let mut s = Scan::fresh(b, from, state, out, Push);
    s.load_state(state);
    s.lex();
    *state = State::keep(&s.stack, s.depth, s.exact, 0);
}

/// Whether a span of kind `kind` is a context's text (a comment's, a string's, a regex
/// literal's, a regex class's in any host), or a template's, which Babel matches a character at
/// a time, not a token: an HTML embed's escape cuts text, where a token that started before it
/// runs past it (a `//` comment takes a `</script>` after it, `\"` an attribute's quote).
pub(crate) fn is_text(kind: u16) -> bool {
    matches!(kind, DOC_TEXT | BLOCK_TEXT | STR | RE | JSTR | TPL_TEXT)
        || kind
            .checked_sub(RE_KINDS)
            .is_some_and(|k| regex::is_set_text(k % regex::KINDS as u16))
}

/// [`line`]'s runs by theme rule pushed to `runs` (X-058, X-068's sink): [`crate::lex_runs`]
/// for JavaScript.
#[inline(never)]
pub fn line_runs(
    b: &[u8],
    state: &mut State,
    out: &mut Out,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
) {
    let mut s = Scan::fresh(b, 0, state, out, Runs::new(rules, runs));
    s.load_state(state);
    s.lex();
    *state = State::keep(&s.stack, s.depth, s.exact, 0);
    s.k.end_line(b.len());
}

/// A block's lines on one scan opened from `state` ([`crate::Carry`], X-067), `state` left as
/// the last one leaves it.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", 0, state, &mut out, Spans::new());
    s.load_state(state);
    s.k.open(s.out, 64);
    let rest = pass(&mut s);
    *state = State::keep(&s.stack, s.depth, s.exact, 0);
    rest
}

/// [`block`]'s runs by theme rule pushed to `runs` ([`crate::CarryRuns`], X-058), the lines'
/// loop compiled with the scan ([`crate::RunLines`], X-086).
pub fn block_runs(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    rules: &mut dyn Rules,
    runs: &mut Vec<Run>,
    lines: &mut crate::RunLines,
) -> Option<usize> {
    let mut out = Out::new(spans);
    out.0.clear();
    let mut s = Scan::fresh(b"", 0, state, &mut out, Runs::new(rules, runs));
    s.load_state(state);
    let rest = lines.pass(&mut s);
    *state = State::keep(&s.stack, s.depth, s.exact, 0);
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

impl crate::CarryRuns for Scan<'_, '_, Runs<'_>> {
    fn next(&mut self, b: &[u8]) -> usize {
        self.k.start_line();
        // SAFETY: as above
        self.carry(unsafe { crate::line_for(b) });
        self.k.end_line(b.len())
    }
}

impl<'b, K: Sink + Loop> Scan<'b, '_, K> {
    /// `b`, the block's next line, lexed on this scan; the scan's line an empty one again after.
    #[inline(always)]
    fn carry(&mut self, b: &'b [u8]) {
        self.b = b;
        // the tags read only below their count
        self.tag_count = 0;
        self.low = self.depth;
        self.text = PLAIN;
        self.at = 0;
        self.loop_at = (usize::MAX, 0);
        self.looped = false;
        self.stuck = 0;
        self.names.set((usize::MAX, NO_NAME));
        self.parens.set((usize::MAX, None));
        // what a long line keeps (X-078), which a short one doesn't read
        if b.len() > LONG_LINE {
            self.paren_read.set(0);
            self.run.set((usize::MAX, 0));
        }
        self.member = usize::MAX;
        self.lex();
        // the line's end as a state's save and load leave it (`line` leaves that to its state)
        crate::stack::hold(&mut self.stack, &mut self.depth, &mut self.exact);
        self.b = b"";
    }
}

/// A [`Scan::names`] entry for no place.
const NO_NAME: Name = Name {
    end: 0,
    next: None,
    word_end: 0,
    mask: 0,
};

impl<'b, 'c, K: Sink + Loop> Scan<'b, 'c, K> {
    /// A scan of `b`, its spans from `from`, returned as a literal so that it is built in place;
    /// the state is [`Scan::load_state`]'s.
    #[inline(always)]
    fn fresh(b: &'b [u8], from: usize, state: &State, out: &'b mut Out<'c>, k: K) -> Self {
        Scan {
            b,
            stack: [MAIN; MAX],
            // read only below `tag_count`
            tags: [(0, 0, 0); TAGS],
            tag_count: 0,
            depth: 1,
            cur: MAIN,
            low: 1,
            exact: state.is_exact(),
            text: PLAIN,
            at: from,
            loop_at: (usize::MAX, 0),
            looped: false,
            stuck: 0,
            names: std::cell::Cell::new((usize::MAX, NO_NAME)),
            parens: std::cell::Cell::new((usize::MAX, None)),
            paren_read: std::cell::Cell::new(0),
            paren_ends: std::cell::RefCell::new(Vec::new()),
            openers: std::cell::RefCell::new(Vec::new()),
            sets: std::cell::Cell::new(0),
            pending: [const { std::mem::MaybeUninit::uninit() }; code::PENDING],
            npending: 0,
            prefix: None,
            slow_at: usize::MAX,
            run: std::cell::Cell::new((usize::MAX, 0)),
            member: usize::MAX,
            ahead: std::cell::Cell::new((UNREAD, UNREAD, UNREAD)),
            k,
            out,
        }
    }

    /// What [`Scan::fresh`] leaves to `state`: the stack and what is read from it, apart from
    /// the literal, which a constructor that changed it before returning it copied (a `memcpy`
    /// a line, X-056's trap).
    #[inline(always)]
    fn load_state(&mut self, state: &State) {
        // the levels kept, whole: those above the depth are never read
        self.depth = state.load(&mut self.stack);
        self.cur = self.stack[self.depth - 1];
        self.low = self.depth;
    }

    /// The line lexed from the stack it starts with.
    #[inline(always)]
    fn lex(&mut self) {
        self.ahead.set((UNREAD, UNREAD, UNREAD));
        self.npending = 0;
        self.prefix = None;
        self.slow_at = usize::MAX;
        self.refresh();
        self.run();
        self.fill(self.b.len());
    }
}

/// A tag's name as a JSX element's context keeps it between lines: 7 bits of its FNV-1a hash.
fn tag_hash(name: &[u8]) -> u8 {
    let mut h: u32 = 0x811c_9dc5;
    for &c in name {
        h = (h ^ u32::from(c)).wrapping_mul(0x0100_0193);
    }
    ((h ^ (h >> 7) ^ (h >> 14) ^ (h >> 21)) & 0x7f) as u8
}

/// Whether context `c` is code whose blanks only `core`'s or `expression`'s patterns can start
/// at: those [`Scan::advance`] skips a run of blanks in.
fn is_code(c: u8) -> bool {
    matches!(
        c,
        MAIN | CURLY
            | ROUND
            | SQUARE
            | SUBST
            | JSX_EXPR
            | SWITCH_BODY
            | CASE
            | PARAM_DEFAULT
            | FOR_IN
            | GQL_SUBST
            | STYLED_ARGS
            | LABEL
            | CLASS_EXTENDS
    )
}

/// `\b` at `p`.
fn boundary(b: &[u8], p: usize) -> bool {
    let before = p > 0 && is_word(b, p - 1);
    before != is_word(b, p)
}

/// Where `[$\w]*` at `p` ends.
fn name_end(b: &[u8], mut p: usize) -> usize {
    loop {
        p = word_end(b, p);
        if p < b.len() && b[p] == b'$' {
            p += 1;
        } else {
            return p;
        }
    }
}

/// `[_$a-zA-Z][$\w]*` at `p`: where it ends.
fn ident(b: &[u8], p: usize) -> Option<usize> {
    let c = *b.get(p)?;
    (c.is_ascii_alphabetic() || c == b'_' || c == b'$').then(|| name_end(b, p + 1))
}

/// Whether `c` can start `[_$a-zA-Z]`.
fn ident_start(c: Option<u8>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphabetic() || c == b'_' || c == b'$')
}

/// `\s` at `p`, the line's `\n` (at its length) included: how many bytes the space takes.
fn space_len(b: &[u8], p: usize) -> Option<usize> {
    if p >= b.len() {
        return (p == b.len()).then_some(1);
    }
    let c = b[p];
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r').then_some(1);
    }
    wide_space(b, p)
}

/// Where `\s*` at `p` ends (it takes the line's `\n`).
fn spaces(b: &[u8], mut p: usize) -> usize {
    while let Some(k) = space_len(b, p) {
        p += k;
    }
    p
}

/// Whether the character before `p` is `\s` (not the line's `\n`, which ends it).
fn space_before(b: &[u8], p: usize) -> Option<usize> {
    if p == 0 || p > b.len() + 1 {
        return None;
    }
    if p == b.len() + 1 {
        // the line's `\n`
        return Some(p - 1);
    }
    let c = b[p - 1];
    if c < 0x80 {
        return matches!(c, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r').then_some(p - 1);
    }
    let s = (p.saturating_sub(4)..p).find(|&s| b[s] & 0xc0 != 0x80)?;
    (space_len(b, s) == Some(p - s)).then_some(s)
}

/// The byte at `i`, the line's `\n` at its length.
fn at(b: &[u8], i: usize) -> Option<u8> {
    match i.cmp(&b.len()) {
        std::cmp::Ordering::Less => Some(b[i]),
        std::cmp::Ordering::Equal => Some(b'\n'),
        std::cmp::Ordering::Greater => None,
    }
}

/// The byte before `p`, the line's `\n` included.
fn before(b: &[u8], p: usize) -> Option<u8> {
    p.checked_sub(1).and_then(|i| at(b, i))
}

/// Whether `$` holds at `i`: at the line's `\n` or after it.
fn eol(b: &[u8], i: usize) -> bool {
    i >= b.len()
}

/// Whether `s` is at `i`.
fn has(b: &[u8], i: usize, s: &[u8]) -> bool {
    crate::has_at(b, i, s)
}

/// `(\((?>(?>[^()]+)|\g<-1>)*\))` at `p`: where the balanced parentheses end.
fn balanced(b: &[u8], p: usize) -> Option<usize> {
    if b.get(p) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    for (i, &c) in b[p..].iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(p + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Lines up to this long read ahead from each place as the grammar does; a longer one keeps
/// what its lookaheads read, so that its places don't each read to its end (X-078). The tests
/// keep it on every line.
#[cfg(not(test))]
const LONG_LINE: usize = 1024;
#[cfg(test)]
const LONG_LINE: usize = 0;

/// No `)` closes the `(` ([`paren_ends`]).
const NO_END: u32 = u32::MAX;

/// [`balanced`]'s reads up to this many bytes aren't counted toward its line's budget: a line
/// of them costs at most this many a place (X-078).
const LONG_READ: usize = 128;

/// [`balanced`] for every `(` of `b` in one pass, into `ends` by its place (other places
/// unset): where its parentheses end, or [`NO_END`]. The open ones are a stack linked through
/// their own entries.
#[cold]
#[inline(never)]
fn paren_ends(b: &[u8], ends: &mut Vec<u32>) {
    ends.clear();
    ends.resize(b.len(), NO_END);
    let mut top = NO_END;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => {
                ends[i] = top;
                top = i as u32;
            }
            b')' if top != NO_END => {
                let below = ends[top as usize];
                ends[top as usize] = i as u32 + 1;
                top = below;
            }
            _ => {}
        }
    }
    while top != NO_END {
        let below = ends[top as usize];
        ends[top as usize] = NO_END;
        top = below;
    }
}

/// [`Scan::ahead`] not read yet on the line.
const UNREAD: usize = usize::MAX;
/// [`Scan::ahead`]'s none.
const NONE_AHEAD: usize = usize::MAX - 1;

/// [`Scan::sets`]: [`Scan::openers`] holds every method body's opener on the line.
const SET_BODY: u8 = 1;
/// [`Scan::sets`]: [`Scan::openers`] holds every arrow's opener on the line.
const SET_ARROW: u8 = 2;

/// What precedes a `{` or a `=>`, past ASCII's blanks.
enum Before {
    /// A `)` here.
    Paren(usize),
    /// A comment's end or a character past ASCII: a `)` may hide behind it.
    Unsure,
    None,
}

/// [`Scan::ahead`] for `b`: each `{` and `=>` found 8 bytes a step, the `)` before it (past
/// ASCII's blanks) the bound; past a comment or a character past ASCII, the `{` or `=` itself;
/// and the last `=>`. With it [`Scan::sets`]'s bits, and `set` filled as [`Scan::openers`]: each
/// `)` matched back to its `(`, a read within twice the line's length (else no set holds).
#[inline(never)]
fn read_ahead(b: &[u8], set: &mut Vec<u32>) -> ((usize, usize, usize), u8) {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGHS: u64 = 0x8080_8080_8080_8080;
    let found = |x: u64, c: u8| {
        let y = x ^ (ONES * u64::from(c));
        y.wrapping_sub(ONES) & !y & HIGHS
    };
    let before = |i: usize| -> Before {
        let mut j = i;
        while j > 0 && matches!(b[j - 1], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') {
            j -= 1;
        }
        match j.checked_sub(1).map(|k| b[k]) {
            Some(b')') => Before::Paren(j - 1),
            Some(b'/' | 0x80..) => Before::Unsure,
            _ => Before::None,
        }
    };
    set.clear();
    let mut exact = if b.len() < (u32::MAX / 4) as usize {
        SET_BODY | SET_ARROW
    } else {
        0
    };
    let (mut arrow, mut body, mut last) = (NONE_AHEAD, NONE_AHEAD, NONE_AHEAD);
    let mut at = |k: usize| match b[k] {
        b'{' => match before(k) {
            Before::Paren(c) => {
                body = c;
                set.push((c as u32) << 1);
            }
            Before::Unsure => {
                body = k;
                exact &= !SET_BODY;
            }
            Before::None => {}
        },
        b'>' if k > 0 && b[k - 1] == b'=' => {
            last = k - 1;
            match before(k - 1) {
                Before::Paren(c) => {
                    arrow = c;
                    set.push((c as u32) << 1 | 1);
                }
                Before::Unsure => {
                    arrow = k - 1;
                    exact &= !SET_ARROW;
                }
                Before::None => {}
            }
        }
        _ => {}
    };
    let mut i = 0;
    while i + 8 <= b.len() {
        let x = u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let mut m = found(x, b'{') | found(x, b'>');
        while m != 0 {
            at(i + (m.trailing_zeros() / 8) as usize);
            m &= m - 1;
        }
        i += 8;
    }
    while i < b.len() {
        at(i);
        i += 1;
    }
    if exact != 0 && !set.is_empty() {
        // each `)` matched back to its `(`: the `(` whose balanced read ends there
        let mut budget = 2 * b.len() + 64;
        for e in set.iter_mut() {
            let c = (*e >> 1) as usize;
            let mut depth = 0usize;
            let mut o = usize::MAX;
            let mut k = c + 1;
            while k > 0 {
                k -= 1;
                match b[k] {
                    b')' => depth += 1,
                    b'(' => {
                        depth -= 1;
                        if depth == 0 {
                            o = k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let spent = c + 1 - k;
            if spent > budget {
                exact = 0;
                break;
            }
            budget -= spent;
            *e = if o == usize::MAX {
                u32::MAX
            } else {
                (o as u32) << 1 | (*e & 1)
            };
        }
        set.retain(|&e| e != u32::MAX);
        set.sort_unstable();
    }
    ((arrow, body, last), exact)
}

/// `(?=(balanced)\s*(=>))` at `p`.
fn arrow_follows_in(b: &[u8], end: Option<usize>) -> bool {
    end.is_some_and(|e| has(b, spaces(b, e), b"=>"))
}

/// `(?=(balanced)(?>\s|/\*.*\*/)*\{)` at `p`: a method's body after its parameters.
fn body_follows_in(b: &[u8], end: Option<usize>) -> bool {
    let Some(mut q) = end else {
        return false;
    };
    loop {
        if let Some(k) = space_len(b, q) {
            q += k;
        } else if has(b, q, b"/*")
            && let Some(i) = b[q + 2..].windows(2).rposition(|w| w == b"*/")
        {
            // `.*` takes the line's last `*/`, and the atomic group keeps it
            q += 2 + i + 2;
        } else {
            return at(b, q) == Some(b'{');
        }
    }
}

/// `function`'s asterisk, its name's start and end, and where the match ends.
type FunctionTail = (Option<usize>, Option<(usize, usize)>, usize);

/// A support object's members: the words, and their kind.
type Members = &'static [(&'static [&'static [u8]], u16)];

/// A key, `(?>'…'|"…"|([_$a-zA-Z][$\w]*|\d+))`: where it ends, and its quote if quoted.
#[derive(Clone, Copy)]
struct Key {
    end: usize,
    quote: Option<u8>,
}

/// A quoted string at `p`, `'(?>[^'\\]|\\.)*'` (or `"`): where it ends.
fn quoted(b: &[u8], p: usize) -> Option<usize> {
    let q = *b.get(p)?;
    let mut i = p + 1;
    loop {
        let c = *b.get(i)?;
        if c == q {
            return Some(i + 1);
        }
        if c == b'\\' {
            // `\\.`: a character, not the line's `\n`
            let &d = b.get(i + 1)?;
            i += 1 + char_len(d);
        } else {
            i += 1;
        }
    }
}

fn key(b: &[u8], p: usize) -> Option<Key> {
    match *b.get(p)? {
        q @ (b'\'' | b'"') => Some(Key {
            end: quoted(b, p)?,
            quote: Some(q),
        }),
        _ => {
            let end = ident(b, p).or_else(|| digits(b, p))?;
            Some(Key { end, quote: None })
        }
    }
}

/// `\d+` at `p`, Unicode's decimal digits: where it ends.
fn digits(b: &[u8], p: usize) -> Option<usize> {
    let mut e = digit(b, p)?;
    while let Some(d) = digit(b, e) {
        e = d;
    }
    Some(e)
}

/// `[0-9]+` at `p`: where it ends.
fn ascii_digits(b: &[u8], p: usize) -> Option<usize> {
    let e = p + b
        .get(p..)?
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    (e > p).then_some(e)
}

/// The words of bat's alternations, each matched as a whole word.
const BUILTIN_CLASSES: &[&[u8]] = &[
    b"Array",
    b"ArrayBuffer",
    b"Boolean",
    b"DataView",
    b"Date",
    b"Float32Array",
    b"Float64Array",
    b"Function",
    b"Infinity",
    b"Int16Array",
    b"Int32Array",
    b"Int8Array",
    b"JSON",
    b"Map",
    b"Math",
    b"NaN",
    b"Number",
    b"Object",
    b"Promise",
    b"Proxy",
    b"Reflect",
    b"RegExp",
    b"Set",
    b"String",
    b"Symbol",
    b"System",
    b"TypeError",
    b"Uint16Array",
    b"Uint32Array",
    b"Uint8Array",
    b"Uint8ClampedArray",
    b"WeakMap",
    b"WeakSet",
];
const ERRORS: &[&[u8]] = &[
    b"Error",
    b"EvalError",
    b"RangeError",
    b"ReferenceError",
    b"SyntaxError",
    b"TypeError",
    b"URIError",
];
const BUILTIN_FNS: &[&[u8]] = &[
    b"constructor",
    b"decodeURI",
    b"decodeURIComponent",
    b"encodeURI",
    b"encodeURIComponent",
    b"escape",
    b"eval",
    b"hasOwnProperty",
    b"isFinite",
    b"isNaN",
    b"isPrototypeOf",
    b"parseFloat",
    b"parseInt",
    b"propertyIsEnumerable",
    b"toLocaleString",
    b"toString",
    b"unescape",
    b"valueOf",
];
const NODE_FNS: &[&[u8]] = &[
    b"clearImmediate",
    b"clearInterval",
    b"clearTimeout",
    b"require",
    b"setImmediate",
    b"setInterval",
    b"setTimeout",
];
const CONSOLE_FNS: &[&[u8]] = &[
    b"assert",
    b"count",
    b"dir",
    b"error",
    b"group",
    b"groupCollapsed",
    b"groupEnd",
    b"info",
    b"log",
    b"profile",
    b"profileEnd",
    b"table",
    b"time",
    b"timeEnd",
    b"trace",
    b"warn",
];
const PROCESS_PROPS: &[&[u8]] = &[
    b"arch",
    b"argv",
    b"config",
    b"env",
    b"execArgv",
    b"execPath",
    b"exitCode",
    b"mainModule",
    b"pid",
    b"platform",
    b"stderr",
    b"stdin",
    b"stdout",
    b"title",
    b"version",
    b"versions",
];
const PROCESS_FNS: &[&[u8]] = &[
    b"abort",
    b"chdir",
    b"cwd",
    b"exit",
    b"getgid",
    b"getgroups",
    b"getuid",
    b"hrtime",
    b"initgroups",
    b"kill",
    b"memoryUsage",
    b"nextTick",
    b"setgid",
    b"setgroups",
    b"setuid",
    b"umask",
    b"uptime",
];
const MODULE_PROPS: &[&[u8]] = &[
    b"children",
    b"exports",
    b"filename",
    b"id",
    b"loaded",
    b"parent",
];

/// `support-class` can take the word.
const W_SUPPORT_CLASS: u16 = 1;
/// `support-other` can take the word (a support object's included).
const W_SUPPORT_OTHER: u16 = 1 << 1;
/// `literal-keywords` can take the word.
const W_KEYWORD: u16 = 1 << 2;
/// `literal-module` can take the word.
const W_MODULE: u16 = 1 << 3;
/// `literal-class` can take the word.
const W_CLASS: u16 = 1 << 4;
/// `flowtype-declaration` can take the word.
const W_FLOW: u16 = 1 << 5;
/// `literal-number` can take the word (`Infinity`).
const W_NUMBER: u16 = 1 << 6;
/// `literal-language-constant` can take the word.
const W_CONSTANT: u16 = 1 << 7;
/// `literal-language-variable` can take the word.
const W_LANG_VAR: u16 = 1 << 8;
/// `literal-constructor` can take the word (`new`).
const W_NEW: u16 = 1 << 9;
/// `literal-operators` can take the word.
const W_OPERATOR: u16 = 1 << 10;
/// A word some other pattern at a name's start looks for: `async`, `function`, `static`,
/// `get`, `set`.
const W_OTHER: u16 = 1 << 11;
/// `literal-for` or `literal-switch` can take the word.
const W_FOR_SWITCH: u16 = 1 << 12;
/// `styled-components` can take the word.
const W_STYLED: u16 = 1 << 13;
/// `graphql` can take the word (`gql`).
const W_GQL: u16 = 1 << 14;

/// Which of the patterns at a name's start can take `w`, a whole `\w` run: a bit each
/// (`W_*`), none for a word no pattern looks for. A name that has none and that none of `=:(`.[`
/// follows can only be `literal-variable`'s.
fn word_mask(w: &[u8]) -> u16 {
    code::info(w).mask
}

/// The keywords a regex literal can follow, `\Wnew|…|^yield`.
const BEFORE_REGEX: &[&[u8]] = &[
    b"new",
    b"delete",
    b"void",
    b"typeof",
    b"instanceof",
    b"in",
    b"do",
    b"return",
    b"case",
    b"throw",
    b"yield",
];

/// A string escape at `p`, a `\`:
/// `\\([1-7][0-7]{0,2}|[0-7]{2,3}|[bfnrtv0'"\\]|x\h{2}|u\{\h+\}|u\h{4})`: where it ends.
fn escape(b: &[u8], p: usize) -> Option<usize> {
    let oct = |i: usize| b.get(i).is_some_and(|c| matches!(c, b'0'..=b'7'));
    let hex = |i: usize| b.get(i).is_some_and(u8::is_ascii_hexdigit);
    let c = *b.get(p + 1)?;
    let q = p + 2;
    match c {
        b'1'..=b'7' => {
            let mut e = q;
            while e < q + 2 && oct(e) {
                e += 1;
            }
            Some(e)
        }
        b'0' if oct(q) => Some(if oct(q + 1) { q + 2 } else { q + 1 }),
        b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'0' | b'\'' | b'"' | b'\\' => Some(q),
        b'x' if hex(q) && hex(q + 1) => Some(q + 2),
        b'u' if b.get(q) == Some(&b'{') && hex(q + 1) => {
            let mut e = q + 1;
            while hex(e) {
                e += 1;
            }
            (b.get(e) == Some(&b'}')).then_some(e + 1)
        }
        b'u' if (q..q + 4).all(hex) => Some(q + 4),
        _ => None,
    }
}

/// The bytes a pattern of a comment, a string, a JSX string, `{{ }}`, a template whose text is
/// plain, and a regex literal can start with.
static COMMENT_STOPS: [bool; 256] = stops(b"*");
static DQ_STOPS: [bool; 256] = stops(b"\"\\");
static SQ_STOPS: [bool; 256] = stops(b"'\\");
static JDQ_STOPS: [bool; 256] = stops(b"\"&");
static JSQ_STOPS: [bool; 256] = stops(b"'&");
static MUSTACHE_STOPS: [bool; 256] = stops(b"}");
static TEMPLATE_STOPS: [bool; 256] = stops(b"`$");
static REGEX_STOPS: [bool; 256] = stops(b"/");
/// A template's, with its escapes: where its patterns start in a regex template.
static TEMPLATE_RE_STOPS: [bool; 256] = stops(b"`$\\");

/// A regex literal's body: its `/` ends it where no group or class is open.
static RE_EMBED: regex::Embed<'static> = regex::Embed {
    dialect: regex::Dialect::Ecma,
    cooked: false,
    yields: &REGEX_STOPS,
    stop: |_, _, _| false,
    arg: 0,
    escape: regex::java_escape,
    top_only: true,
    continues: false,
    base: RE_KINDS,
    text: RE,
};

/// A regex in a `"…"`, a `'…'` and a template ([`RE_STR`]): the host's end, its escapes and a
/// template's `${` come first.
static RE_STR_EMBED: [regex::Embed<'static>; 3] = [
    re_str_embed(&DQ_STOPS, 1, STR),
    re_str_embed(&SQ_STOPS, 1, STR),
    regex::Embed {
        stop: |b, p, _| b[p] == b'`' || b.get(p + 1) == Some(&b'{'),
        // a `\` that isn't one of Babel's escapes is the `\` alone (`\${` is a substitution)
        escape: |b, p| {
            escape(b, p).unwrap_or(if b.get(p + 1) == Some(&b'`') {
                p + 2
            } else {
                p + 1
            })
        },
        ..re_str_embed(&TEMPLATE_RE_STOPS, 2, TPL_TEXT)
    },
];

const fn re_str_embed(yields: &'static [bool; 256], host: u16, text: u16) -> regex::Embed<'static> {
    regex::Embed {
        dialect: regex::Dialect::Ecma,
        cooked: true,
        yields,
        stop: |_, _, _| true,
        arg: 0,
        escape: |b, p| {
            escape(b, p).unwrap_or_else(|| b.get(p + 1).map_or(p + 1, |&d| p + 1 + char_len(d)))
        },
        top_only: false,
        continues: true,
        base: RE_KINDS + host * regex::KINDS as u16,
        text,
    }
}

/// Whether a sink's scan runs the code frames' own loop ([`Scan::code_run`]): not HTML's embed
/// (`Push`), whose copy of the scan stays the dispatch's alone, in ada beside the run copy.
trait Loop {
    const LOOP: bool;
}

impl Loop for Push {
    const LOOP: bool = false;
}

impl Loop for Spans {
    const LOOP: bool = true;
}

impl Loop for Runs<'_> {
    const LOOP: bool = true;
}

struct Scan<'b, 'c, K: Sink> {
    /// The line, without its `\n`: its length is where the `\n` is.
    b: &'b [u8],
    stack: [u8; MAX],
    /// For a JSX element's context pushed on this line, where its tag's name is.
    tags: [(u8, u32, u32); TAGS],
    /// How many tags were pushed on this line: the last [`TAGS`] are in `tags`, by count.
    tag_count: usize,
    depth: usize,
    /// The top context, `stack[depth - 1]`, read without the stack (X-058).
    cur: u8,
    /// The lowest depth the stack has had on this line: levels from it up were pushed on it.
    low: usize,
    exact: bool,
    /// The kind of text no pattern of the top context matches.
    text: u16,
    /// Where the spans so far end.
    at: usize,
    /// Where the last push that matched nothing was, and the depth it left: a pop there, at
    /// that depth, that matches nothing would loop, so syntect tries the patterns after it and
    /// else steps a character.
    loop_at: (usize, usize),
    /// A pop at the place was passed over (`loop_at`).
    looped: bool,
    /// Matches that took nothing, in a row at one place (reset when the scan moves).
    stuck: usize,
    /// The last name read ([`Scan::name_at`]): where, and what.
    names: std::cell::Cell<(usize, Name)>,
    /// The last `(` [`Scan::balanced`] was asked about, and its answer.
    parens: std::cell::Cell<(usize, Option<usize>)>,
    /// The bytes [`balanced`] has read on this line in reads past [`LONG_READ`]: past a budget,
    /// the line's parentheses are matched once, into `paren_ends`, and this is `usize::MAX`
    /// (X-078).
    paren_read: std::cell::Cell<usize>,
    /// Where the parentheses opened at each `(` of the line end ([`paren_ends`]), kept across a
    /// block's lines for its allocation.
    paren_ends: std::cell::RefCell<Vec<u32>>,
    /// The `(`s whose `)` a `{` follows (`(` at `2i`) or a `=>` (`2i + 1`), sorted, where
    /// [`Scan::sets`] says the line's are all known (X-058): a method's or an arrow's opener is
    /// one of them, with no balanced read from it.
    openers: std::cell::RefCell<Vec<u32>>,
    /// Which of [`Scan::openers`]' sets hold on the line: [`SET_BODY`], [`SET_ARROW`].
    sets: std::cell::Cell<u8>,
    /// The groups opened on this line whose `(` may turn out an arrow's or a method's, decided
    /// at their `)` ([`code::Pending`], X-058): those below [`Scan::npending`].
    pending: [std::mem::MaybeUninit<code::Pending>; code::PENDING],
    npending: usize,
    /// An assignment's `name =` whose value starts with the `(` at `.0`: that group's undo
    /// point is before the name ([`code::Prefix`]).
    prefix: Option<code::Prefix>,
    /// A place the code frames' loop hands straight to the dispatch, once (a group's `(` or a
    /// name taken back).
    slow_at: usize,
    /// The last run of `\s` read from a place ([`Scan::spaces_at`]): where it starts and ends.
    run: std::cell::Cell<(usize, usize)>,
    /// Where the last member after a `.` [`Scan::after_object`] took starts.
    member: usize,
    /// Where the line's last `)` before `\s*=>` and its last before `\s*{` (or a comment) are, at
    /// most ([`NONE_AHEAD`] for none), read once a line when first asked ([`UNREAD`] till then):
    /// an arrow's parameters and a method's close at neither past them (X-058); and where its
    /// last `=>` is.
    ahead: std::cell::Cell<(usize, usize, usize)>,
    /// Where the lexemes go: a line's spans, as [`Out`] pushes them, or its runs by theme rule
    /// (X-058, X-068's sink).
    k: K,
    out: &'b mut Out<'c>,
}

/// A name at a place, as the patterns at its start look at it.
#[derive(Clone, Copy)]
struct Name {
    /// Where `[$\w]*` after its first character ends.
    end: usize,
    /// The byte after the spaces after it.
    next: Option<u8>,
    /// Where its `\w` run ends.
    word_end: usize,
    /// Which patterns at a name's start can take its `\w` run ([`word_mask`]).
    mask: u16,
}

impl<'b, K: Sink + Loop> Scan<'b, '_, K> {
    #[inline(always)]
    fn top(&self) -> u8 {
        self.cur
    }

    /// The top context replaced by `c`, in place (a regex's state).
    #[inline(always)]
    fn set_top(&mut self, c: u8) {
        self.stack[self.depth - 1] = c;
        self.cur = c;
    }

    fn refresh(&mut self) {
        self.text = match self.top() {
            DOC => DOC_TEXT,
            BLOCK => BLOCK_TEXT,
            STR_D | STR_S => STR,
            REGEX..=RE_LAST => RE,
            JSTR_D | JSTR_S => JSTR,
            _ => PLAIN,
        };
    }

    /// The byte at `i`, the line's `\n` at its length.
    fn c(&self, i: usize) -> Option<u8> {
        at(self.b, i)
    }

    fn is(&self, i: usize, c: u8) -> bool {
        self.c(i) == Some(c)
    }

    /// `(?<!\.)`.
    fn not_after_dot(&self, p: usize) -> bool {
        before(self.b, p) != Some(b'.')
    }

    /// The whole word at `p`, `\b(\w+)\b`, when one starts there.
    fn word(&self, p: usize) -> Option<&'b [u8]> {
        if p >= self.b.len() || !boundary(self.b, p) || !is_word(self.b, p) {
            return None;
        }
        // the name read there ([`Scan::name_at`]) knows where its `\w` run ends
        let (at, name) = self.names.get();
        let we = if at == p {
            name.word_end
        } else {
            word_end(self.b, p)
        };
        Some(&self.b[p..we])
    }

    /// `(?<!\.)\b(?>…)\b` for one of `words`: where it ends.
    fn keyword(&self, p: usize, words: &[&[u8]]) -> Option<usize> {
        if !self.not_after_dot(p) {
            return None;
        }
        let w = self.word(p)?;
        words.contains(&w).then(|| p + w.len())
    }

    /// The text up to `p` is the top context's.
    fn fill(&mut self, p: usize) {
        let text = self.text;
        self.fill_as(p, text);
    }

    /// The text up to `p` (past the line's end, to it) is `text`: one call on the run sink
    /// ([`Sink::merge_call`]), these writes being inlined at many places.
    #[inline(always)]
    fn fill_as(&mut self, p: usize, text: u16) {
        let p = p.min(self.b.len());
        if p > self.at {
            if text != PLAIN {
                self.k.merge_call(&mut self.out, self.at, p, text);
            }
            self.at = p;
        }
    }

    /// Bytes `s..e` are `leaf`, after the text before them.
    fn put(&mut self, s: usize, e: usize, leaf: u16) {
        self.fill(s);
        self.fill_as(e, leaf);
    }

    fn push(&mut self, c: u8) {
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth, &mut self.exact);
            // the levels moved: the tags' places no longer name them, nor the groups' theirs
            self.tag_count = 0;
            self.npending = 0;
            self.tags = [(0, 0, 0); TAGS];
            self.low = self.low.saturating_sub(1).max(1);
        }
        self.stack[self.depth] = c;
        self.depth += 1;
        self.cur = c;
        self.refresh();
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
            self.cur = self.stack[self.depth - 1];
        }
        self.low = self.low.min(self.depth);
        self.refresh();
    }

    /// Whether a push may match: not at syntect's depth limit.
    fn can_push(&self) -> bool {
        self.depth < SYNTECT_DEPTH
    }

    /// `c` pushed by a match that took nothing at `p`: syntect keeps the depth it leaves for
    /// its guard against a loop.
    fn enter(&mut self, p: usize, c: u8) -> Option<usize> {
        if !self.can_push() {
            return None;
        }
        self.fill(p);
        self.push(c);
        self.loop_at = (p, self.depth);
        Some(p)
    }

    /// A pop that takes nothing at `p`, unless it would loop: then syntect passes over it for
    /// the patterns after it (`None`).
    fn leave(&mut self, p: usize) -> Option<usize> {
        if self.loop_at == (p, self.depth) {
            self.looped = true;
            return None;
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    /// Bytes `s..e` are `leaf`; then `c` is pushed.
    fn tok_push(&mut self, s: usize, e: usize, leaf: u16, c: u8) -> Option<usize> {
        if !self.can_push() {
            return None;
        }
        self.put(s, e, leaf);
        self.push(c);
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, then the top context pops.
    fn close(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        self.put(s, e, leaf);
        self.pop();
        Some(e)
    }

    /// Bytes `s..e` are `leaf`, and the context stays.
    fn tok(&mut self, s: usize, e: usize, leaf: u16) -> Option<usize> {
        self.put(s, e, leaf);
        Some(e)
    }

    fn run(&mut self) {
        let end = self.b.len() + 1;
        let mut p = self.at;
        loop {
            // the code frames' own loop first (X-058), where one is on top
            if K::LOOP && p < self.b.len() && code::is_fast(self.top()) {
                let q = self.code_run(p);
                if q != p {
                    self.stuck = 0;
                    p = q;
                }
            }
            // at the line's `\n` and past it no pattern of code, of a comment or of a template's
            // text can match (`core` and `expression` take nothing there, nor these contexts'
            // own patterns): the dispatches there would fail
            if p >= self.b.len() && (is_code(self.top()) || matches!(self.top(), DOC | BLOCK | TPL))
            {
                return;
            }
            self.looped = false;
            // a run of blanks in code where no pattern can start, skipped whole: none of these
            // contexts' own patterns starts at a blank, and `core`'s and `expression`'s only
            // after a `.` or before `:`, `gql`, `` ` ``, `/` or `<` ([`Scan::blank_may_match`])
            if p < self.b.len() && matches!(self.b[p], b' ' | b'\t') && is_code(self.top()) {
                let q = p + self.b[p..]
                    .iter()
                    .take_while(|&&c| c == b' ' || c == b'\t')
                    .count();
                // past them, a space only `\s` takes ends the shortcut (the line's `\n` doesn't);
                // a comment's `/`, a `g` not of `gql``, a `<` before no name start none
                let next = self.c(q);
                let may = match next {
                    Some(b'\x0b' | b'\x0c' | b'\r' | b':' | b'`' | 0x80..) => true,
                    Some(b'/') => !matches!(self.c(q + 1), Some(b'/' | b'*')),
                    Some(b'g') => has(self.b, q, b"gql`"),
                    Some(b'<') => ident_start(self.c(q + 1)),
                    _ => false,
                };
                if !may && before(self.b, p) != Some(b'.') {
                    p = q;
                    self.stuck = 0;
                    continue;
                }
            }
            if let Some(q) = self.dispatch(p) {
                self.stuck = if q == p { self.stuck + 1 } else { 0 };
                p = q;
                if self.stuck > STUCK {
                    self.stuck = 0;
                    if p >= self.b.len() {
                        return;
                    }
                    p = self.next(p);
                }
            } else if p >= end || (self.looped && p >= self.b.len()) {
                return;
            } else {
                p = self.advance(p);
                self.stuck = 0;
            }
        }
    }

    /// Where the character at `p` ends (the line's `\n` is one byte).
    fn next(&self, p: usize) -> usize {
        crate::prim::next_char(self.b, p)
    }

    /// Where a pattern of the top context can next start, after none did at `p`.
    fn advance(&self, p: usize) -> usize {
        if p >= self.b.len() {
            return p + 1;
        }
        let b = self.b;
        let stops: &[bool; 256] = match self.top() {
            DOC | BLOCK => &COMMENT_STOPS,
            STR_D => &DQ_STOPS,
            STR_S => &SQ_STOPS,
            JSTR_D => &JDQ_STOPS,
            JSTR_S => &JSQ_STOPS,
            MUSTACHE => &MUSTACHE_STOPS,
            STYLED | GRAPHQL => &TEMPLATE_STOPS,
            REGEX..=RE_LAST => &REGEX_STOPS,
            // Flow's brackets and annotation: at a blank only `flowtype-tokens`' `\s*([|&])` and
            // `(?<=[:?|&=])(?=\s*{)` can start, the first at every blank of a run alike, the
            // second only at its first
            FLOW_ANNOT | FLOW_BRACES | FLOW_ANGLE | FLOW_SQUARE | FLOW_ROUND
                if matches!(b[p], b' ' | b'\t') =>
            {
                let q = p + b[p..]
                    .iter()
                    .take_while(|&&c| c == b' ' || c == b'\t')
                    .count();
                return if matches!(
                    self.c(q),
                    Some(b'|' | b'&' | b'\x0b' | b'\x0c' | b'\r' | 0x80..)
                ) {
                    p + 1
                } else {
                    q
                };
            }
            // code: inside a run of blanks only `literal-labels`' `\s*:` can start, and only
            // where a `:` ends the run (its other patterns' lookbehinds see the blank)
            c if is_code(c) && matches!(b[p], b' ' | b'\t') => {
                let q = p + b[p..]
                    .iter()
                    .take_while(|&&c| c == b' ' || c == b'\t')
                    .count();
                return if self.is(q, b':') { p + 1 } else { q };
            }
            _ => return self.next(p),
        };
        let mut q = p + 1;
        while q < self.b.len() && !stops[usize::from(b[q])] {
            q += 1;
        }
        q
    }

    /// The first pattern of the top context that matches at `p`, applied: where the scan goes
    /// on, or `None` when none does.
    fn dispatch(&mut self, p: usize) -> Option<usize> {
        let top = self.top();
        if top & JSX_TAG != 0 {
            return self.jsx_tag(p, top);
        }
        let c = self.c(p);
        match top {
            MAIN => self.core(p),
            CURLY => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, CURLY_END);
                }
                self.core(p)
            }
            ROUND => {
                if c == Some(b')') {
                    return self.close(p, p + 1, ROUND_END);
                }
                self.expression(p)
            }
            SQUARE => {
                if c == Some(b']') {
                    return self.close(p, p + 1, SQUARE_END);
                }
                self.expression(p)
            }
            DOC | BLOCK => {
                if has(self.b, p, b"*/") {
                    let leaf = if top == DOC { DOC_MARK } else { BLOCK_END };
                    return self.close(p, p + 2, leaf);
                }
                None
            }
            STR_D | STR_S => {
                let q = if top == STR_D { b'"' } else { b'\'' };
                if c == Some(q) {
                    return self.close(p, p + 1, STR_END);
                }
                if p == self.b.len() {
                    // `(\n)`: the string ends with its line
                    self.fill(p);
                    self.pop();
                    return Some(p + 1);
                }
                self.string_content(p, STR_ESC, STR_ESC_NL)
            }
            TPL => self.template(p),
            SUBST => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, SUBST_END);
                }
                self.expression(p)
            }
            RE_STR..=RE_STR_LAST => self.regex_string(p, top),
            REGEX..=RE_LAST => {
                if top == REGEX && c == Some(b'/') {
                    let f = p
                        + 1
                        + self.b[p + 1..]
                            .iter()
                            .take_while(|c| {
                                matches!(c, b'd' | b'g' | b'i' | b'm' | b's' | b'u' | b'v' | b'y')
                            })
                            .count();
                    self.put(p, p + 1, RE_END);
                    return self.close(p + 1, f, RE_FLAGS);
                }
                self.regex(p, top)
            }
            FOR => {
                if c == Some(b')') {
                    return self.close(p, p + 1, ROUND_END);
                }
                self.comments(p).or_else(|| {
                    (c == Some(b'('))
                        .then(|| self.tok_push(p, p + 1, ROUND_BEGIN, FOR_IN))
                        .flatten()
                })
            }
            FOR_IN => {
                if c == Some(b')')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.storage(p)
                    .or_else(|| self.expression(p))
                    .or_else(|| self.punctuation(p))
            }
            SWITCH => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, CURLY_END);
                }
                self.comments(p).or_else(|| match c {
                    Some(b'(') => self.tok_push(p, p + 1, ROUND_BEGIN, ROUND),
                    Some(b'{') => self.tok_push(p, p + 1, CURLY_BEGIN, SWITCH_BODY),
                    _ => None,
                })
            }
            SWITCH_BODY => {
                if c == Some(b'}')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                if let Some(e) = self.not_after_dot(p).then(|| self.word(p)).flatten()
                    && (e == b"case" || e == b"default")
                {
                    let e = p + e.len();
                    if let Some(q) = self.tok_push(p, e, SWITCH_KW, CASE) {
                        return Some(q);
                    }
                }
                self.core(p)
            }
            CASE => {
                if c == Some(b':')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.expression(p)
            }
            CLASS => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, CURLY_END);
                }
                self.comments(p)
                    .or_else(|| {
                        let e = ident(self.b, p)?;
                        self.tok_push(p, e, ENTITY_CLASS, CLASS_HEAD)
                    })
                    .or_else(|| {
                        (c == Some(b'{'))
                            .then(|| self.tok_push(p, p + 1, CURLY_BEGIN, CLASS_BODY))
                            .flatten()
                    })
            }
            CLASS_HEAD => {
                if c == Some(b'{')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.comments(p).or_else(|| self.polymorph(p)).or_else(|| {
                    let e = self.keyword(p, &[b"extends"])?;
                    self.tok_push(p, e, EXTENDS, CLASS_EXTENDS)
                })
            }
            CLASS_EXTENDS => {
                if c == Some(b'{')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.polymorph(p).or_else(|| self.expression(p))
            }
            CLASS_BODY => {
                if c == Some(b'}')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.decorator(p)
                    .or_else(|| self.class_property(p))
                    .or_else(|| self.method_definition(p))
                    .or_else(|| self.method_storage(p))
                    .or_else(|| self.brackets(p))
                    .or_else(|| self.comments(p))
            }
            CLASS_PROP => {
                let s = self.spaces_at(p);
                if matches!(self.c(s), Some(b';' | b'}'))
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.flow_annotation(p).or_else(|| self.expression(p))
            }
            METHOD_DEF => {
                let s = self.spaces_at(p);
                if matches!(self.c(s), Some(b';' | b'{'))
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.polymorph(p)
                    .or_else(|| self.parameters(p))
                    .or_else(|| self.flow_annotation(p))
                    .or_else(|| self.comments(p))
                    .or_else(|| {
                        (c == Some(b'{'))
                            .then(|| self.tok_push(p, p + 1, CURLY_BEGIN, CURLY))
                            .flatten()
                    })
            }
            FUNC | METHOD => {
                if before(self.b, p) == Some(b')')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                if top == FUNC
                    && let Some(q) = self.polymorph(p)
                {
                    return Some(q);
                }
                self.parameters(p)
            }
            ARROW => {
                if before(self.b, p) == Some(b')') {
                    let s = self.spaces_at(p);
                    if has(self.b, s, b"=>") {
                        self.put(s, s + 2, ARROW_KW);
                        self.pop();
                        return Some(s + 2);
                    }
                }
                self.parameters(p)
            }
            PARAMS => self.params(p),
            PARAM_DEFAULT => {
                if matches!(c, Some(b',' | b')'))
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.expression(p)
            }
            CTOR => {
                // `(?![_$a-zA-Z][$\w.]*)`
                if !ident_start(c)
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.support_class(p)
                    .or_else(|| self.support_other(p))
                    .or_else(|| self.constructor_name(p))
            }
            LABEL => {
                if c == Some(b':') {
                    return self.close(p, p + 1, KEY_SEP);
                }
                self.string(p)
            }
            JSX => {
                if before(self.b, p) == Some(b'>')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.jsx_tag_start(p)
            }
            JSX_CHILDREN => {
                if has(self.b, p, b"</")
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.jsx_tag_start(p)
                    .or_else(|| self.jsx_evaluated(p))
                    .or_else(|| self.jsx_entities(p, ENTITY, BAD_AMP))
            }
            JSX_EXPR => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, EMBED_END);
                }
                self.expression(p)
            }
            JSTR_D | JSTR_S => {
                let q = if top == JSTR_D { b'"' } else { b'\'' };
                if c == Some(q) {
                    return self.close(p, p + 1, JSTR_END);
                }
                self.jsx_entities(p, JSTR_ENTITY, JSTR_BAD)
            }
            FLOW_ANNOT => {
                if let Some(q) = self.flow_tokens(p) {
                    return Some(q);
                }
                if p < self.b.len()
                    && space_len(self.b, p).is_none()
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                None
            }
            FLOW_BRACES => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, FLOW_END);
                }
                self.flow_tokens(p)
            }
            FLOW_DECL => {
                let s = self.spaces_at(p);
                if (self.is(s, b';') || has(self.b, s, b"from"))
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.flow_tokens(p)
            }
            FLOW_DECL_BRACES => {
                if before(self.b, p) == Some(b'}')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.comments(p)
                    .or_else(|| self.flow_brackets(p))
                    .or_else(|| self.polymorph(p))
                    .or_else(|| {
                        let e = ident(self.b, p)?;
                        self.tok(p, e, ENTITY_TYPE)
                    })
            }
            FLOW_ANGLE | FLOW_SQUARE | FLOW_ROUND => {
                let close = match top {
                    FLOW_ANGLE => b'>',
                    FLOW_SQUARE => b']',
                    _ => b')',
                };
                if c == Some(close) {
                    return self.close(p, p + 1, FLOW_END);
                }
                self.flow_tokens(p)
            }
            FLOW_AFTER => {
                if before(self.b, p) == Some(b'}')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.flow_brackets(p)
            }
            FLOW_FN => {
                if before(self.b, p)
                    .is_some_and(|c| c == b'}' || c.is_ascii_alphabetic() || c == b'_' || c == b'$')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                self.flow_brackets(p)
                    .or_else(|| self.flow_identifier(p))
                    .or_else(|| self.comments(p))
            }
            GRAPHQL => {
                if c == Some(b'`') {
                    self.fill(p);
                    self.pop();
                    return Some(p + 1);
                }
                has(self.b, p, b"${")
                    .then(|| self.tok_push(p, p + 2, PLAIN, GQL_SUBST))
                    .flatten()
            }
            GQL_SUBST => {
                if c == Some(b'}') {
                    return self.close(p, p + 1, PLAIN);
                }
                self.core(p)
            }
            STYLED => {
                if c == Some(b'`') {
                    return self.close(p, p + 1, TPL_MARK_END);
                }
                None
            }
            STYLED_CALL => {
                if before(self.b, p) == Some(b'`')
                    && let Some(q) = self.leave(p)
                {
                    return Some(q);
                }
                match c {
                    Some(b'(') => self.tok_push(p, p + 1, GROUP_BEGIN, STYLED_ARGS),
                    Some(b'`') => self.tok_push(p, p + 1, TPL_MARK_BEGIN, STYLED),
                    _ => None,
                }
            }
            STYLED_ARGS => {
                if c == Some(b')') {
                    return self.close(p, p + 1, GROUP_END);
                }
                self.expression(p)
            }
            MUSTACHE => {
                if has(self.b, p, b"}}") {
                    return self.close(p, p + 2, PLAIN);
                }
                None
            }
            _ => None,
        }
    }

    /// `([_$a-zA-Z][$\w.]*\.)?([_$a-zA-Z][$\w]*)` at `p`: the name after the dotted run's last
    /// `.` that a name follows is the constructor's.
    fn constructor_name(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        ident(b, p)?;
        // `[$\w.]*` from after the first character, then back to a `.` before a name
        let mut run = p + 1;
        loop {
            run = name_end(b, run);
            if run < self.b.len() && b[run] == b'.' {
                run += 1;
            } else {
                break;
            }
        }
        let dot = (p + 1..run)
            .rev()
            .find(|&i| b[i] == b'.' && ident_start(b.get(i + 1).copied()));
        let s = dot.map_or(p, |d| d + 1);
        let e = ident(b, s)?;
        self.put(p, s, PLAIN);
        self.tok(s, e, CTOR_NAME)
    }

    // --- core and expression ---

    /// `core`: `literal-function-labels`, `literal-arrow-function-labels`, `literal-labels`,
    /// `literal-for`, `literal-switch`, `styled-components`, `graphql`, `expression`,
    /// `literal-punctuation`.
    fn core(&mut self, p: usize) -> Option<usize> {
        if p >= self.b.len() {
            return None;
        }
        // no pattern before `literal-punctuation`'s can start at `;` or `,`
        match self.b[p] {
            b';' => return self.tok(p, p + 1, TERMINATOR),
            b',' => return self.tok(p, p + 1, COMMA),
            _ => {}
        }
        // only the patterns that can start with the byte at `p`, in the grammar's order
        if let Some(q) = self.plain_name(p, true) {
            return Some(q);
        }
        let r = match self.b[p] {
            // a name or a number is a key only before `:`, and is one of the grammar's words
            // before anything else here
            // at a name or a number, the labels' patterns want a `:` after it, the others
            // their words
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' | b'0'..=b'9' => {
                let n = self.name_at(p);
                let colon = n.next == Some(b':');
                let r = if colon {
                    self.function_labels(p)
                        .or_else(|| self.arrow_labels(p))
                        .or_else(|| self.labels(p))
                } else {
                    None
                };
                r.or_else(|| {
                    if n.mask & W_FOR_SWITCH != 0 {
                        self.for_switch(p)
                    } else if n.mask & W_STYLED != 0 {
                        self.styled(p)
                    } else if n.mask & W_GQL != 0 {
                        self.graphql(p)
                    } else {
                        None
                    }
                })
            }
            // a string the keys' patterns can't take (no `:` after it, nor `literal-method`'s
            // `(`): `literal-string`'s
            b'\'' | b'"' => match quoted(self.b, p).map(|e| self.c(spaces(self.b, e))) {
                Some(Some(b':')) => self
                    .function_labels(p)
                    .or_else(|| self.arrow_labels(p))
                    .or_else(|| self.labels(p)),
                Some(Some(b'(')) => None,
                _ => return self.string(p),
            },
            // past ASCII, only a space or a digit can start one of these
            0x80.. if !self.wide_space_or_digit(p) => None,
            0x80.. => self
                .function_labels(p)
                .or_else(|| self.arrow_labels(p))
                .or_else(|| self.labels(p))
                .or_else(|| self.for_switch(p))
                .or_else(|| self.styled(p))
                .or_else(|| self.graphql(p)),
            b' ' | b'\t' | 0x0b | 0x0c | b'\r' if !self.blank_may_match(p) => return None,
            b' ' | b'\t' | 0x0b | 0x0c | b'\r' | b':' => self.labels(p).or_else(|| self.graphql(p)),
            b'`' => self.graphql(p),
            _ => None,
        };
        r.or_else(|| self.expression(p))
            .or_else(|| self.punctuation(p))
    }

    /// [`balanced`] at `p`, the last answer kept: a call's `(` is asked about by
    /// `literal-method`'s lookahead, then by `literal-arrow-function`'s.
    #[inline(always)]
    fn balanced(&self, p: usize) -> Option<usize> {
        let (at, end) = self.parens.get();
        if at == p {
            return end;
        }
        if self.b.len() > LONG_LINE {
            return self.balanced_kept(p);
        }
        let end = balanced(self.b, p);
        self.parens.set((p, end));
        end
    }

    /// [`Scan::balanced`] on a long line. A `(` that nothing closes is read to the line's end,
    /// so a line of them would be read again from each (X-078): once the line's long reads
    /// pass twice its length, its parentheses are matched in one pass and the answers read
    /// from that.
    #[inline(never)]
    fn balanced_kept(&self, p: usize) -> Option<usize> {
        let end = if self.paren_read.get() == usize::MAX {
            self.balanced_by_table(p)
        } else {
            let end = balanced(self.b, p);
            // a short read costs what a byte of the line does: only the long ones are counted
            // (none at a place that isn't a `(`, which reads nothing)
            let read = match end {
                Some(e) => e - p,
                None if self.b.get(p) == Some(&b'(') => self.b.len() - p,
                None => 0,
            };
            if read > LONG_READ {
                self.long_read(read);
            }
            end
        };
        self.parens.set((p, end));
        end
    }

    /// [`Scan::balanced`] past its budget: the answer from the line's table.
    #[cold]
    #[inline(never)]
    fn balanced_by_table(&self, p: usize) -> Option<usize> {
        if self.b.get(p) != Some(&b'(') {
            return None;
        }
        let e = self.paren_ends.borrow()[p];
        (e != NO_END).then_some(e as usize)
    }

    /// A read of [`Scan::balanced`]'s past [`LONG_READ`], counted: once the line's pass twice
    /// its length, its parentheses are matched into `paren_ends` ([`paren_ends`]) and the count
    /// is `usize::MAX`; a line too long for the table's places reads on.
    #[cold]
    #[inline(never)]
    fn long_read(&self, read: usize) {
        let read = self.paren_read.get() + read;
        if read > 2 * self.b.len() && self.b.len() < NO_END as usize {
            paren_ends(self.b, &mut self.paren_ends.borrow_mut());
            self.paren_read.set(usize::MAX);
        } else {
            self.paren_read.set(read);
        }
    }

    /// `spaces` at `p`, the place the patterns are tried at.
    #[inline(always)]
    fn spaces_at(&self, p: usize) -> usize {
        let b = self.b;
        if b.len() > LONG_LINE {
            return self.spaces_kept(p);
        }
        spaces(b, p)
    }

    /// [`Scan::spaces_at`] on a long line, a run read once: the patterns tried at each place
    /// along a run of blanks read it ahead from there (`\s*:`, `\s*gql`, `\s*/`, …), and a line
    /// of them would be read again from each (X-078).
    #[inline(never)]
    fn spaces_kept(&self, p: usize) -> usize {
        let b = self.b;
        if p < b.len() && b[p] < 0x80 && !matches!(b[p], b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r') {
            return p;
        }
        let (s, e) = self.run.get();
        // a place in the run that starts a character is one its read passed
        if s <= p && p < e && (p >= b.len() || b[p] & 0xc0 != 0x80) {
            return e;
        }
        let e = spaces(b, p);
        self.run.set((p, e));
        e
    }

    /// `(?=(…)\s*(=>))` at `p`.
    fn arrow_follows(&self, p: usize) -> bool {
        if !self.is(p, b'(') || !self.ahead_of(p).0 {
            return false;
        }
        if self.sets.get() & SET_ARROW != 0 {
            return self.opens(p, 1);
        }
        arrow_follows_in(self.b, self.balanced(p))
    }

    /// Whether the line's last `)` before `=>`, and its last before `{`, are past `p`.
    #[inline(always)]
    fn ahead_of(&self, p: usize) -> (bool, bool, bool) {
        let (mut a, mut c, mut d) = self.ahead.get();
        if a == UNREAD {
            let x;
            ((a, c, d), x) = read_ahead(self.b, &mut self.openers.borrow_mut());
            self.ahead.set((a, c, d));
            self.sets.set(x);
        }
        (
            a != NONE_AHEAD && a > p,
            c != NONE_AHEAD && c > p,
            d != NONE_AHEAD && d > p,
        )
    }

    /// A method's body after its parameters at `p`.
    fn body_follows(&self, p: usize) -> bool {
        if !self.is(p, b'(') || !self.ahead_of(p).1 {
            return false;
        }
        if self.sets.get() & SET_BODY != 0 {
            return self.opens(p, 0);
        }
        body_follows_in(self.b, self.balanced(p))
    }

    /// Whether the `(` at `p` is one of [`Scan::openers`]' set `kind` (0 a method's body, 1 an
    /// arrow's).
    #[inline(always)]
    fn opens(&self, p: usize, kind: u32) -> bool {
        self.openers
            .borrow()
            .binary_search(&((p as u32) << 1 | kind))
            .is_ok()
    }

    /// The name at `p`, read once.
    fn name_at(&self, p: usize) -> Name {
        let (at, name) = self.names.get();
        if at == p {
            return name;
        }
        let b = self.b;
        // `[$\w]*` is the `\w` run, then more after a `$`
        let we = word_end(b, p);
        let end = if b.get(we) == Some(&b'$') {
            name_end(b, we)
        } else {
            we.max(p + 1)
        };
        let name = Name {
            end,
            next: self.c(spaces(b, end)),
            word_end: we,
            mask: word_mask(&b[p..we]),
        };
        self.names.set((p, name));
        name
    }

    /// A name that no pattern keys on ([`word_mask`]), classified once by the byte after it
    /// instead of through the patterns, each of which would fail on it but the one whose output
    /// this gives: `literal-variable`'s, `literal-method-call`'s and `literal-function-call`'s,
    /// or in `core` (`core`) `literal-labels`'. `None` where a function, an arrow, a method's
    /// body or `prototype` may follow, for the patterns in order.
    fn plain_name(&mut self, p: usize, core: bool) -> Option<usize> {
        let mut q = self.plain_name_one(p, core)?;
        // a member after a `.` this step took ([`Scan::after_object`]): the dispatch at it, in
        // the same context, would start here
        while self.member == q {
            let core = matches!(self.top(), MAIN | CURLY | SWITCH_BODY | GQL_SUBST);
            match self.plain_name_one(q, core) {
                Some(r) => q = r,
                None => break,
            }
        }
        Some(q)
    }

    /// [`Scan::plain_name`] for the name at `p` alone.
    fn plain_name_one(&mut self, p: usize, core: bool) -> Option<usize> {
        let b = self.b;
        // one starting uppercase or with `_` can be a constant or a class
        let upper = b[p].is_ascii_uppercase() || b[p] == b'_';
        if !(upper || b[p].is_ascii_lowercase() || b[p] == b'$') {
            return None;
        }
        let Name {
            end: e, next, mask, ..
        } = self.name_at(p);
        if mask != 0 {
            return if upper {
                None
            } else {
                self.keyword_name(p, e, next, mask, core)
            };
        }
        let after_dot = before(b, p) == Some(b'.');
        // `literal-variable`'s: a property after a `.`, else a variable
        let var = if after_dot { PROPERTY } else { READWRITE };
        match next {
            Some(b'=') => {
                let s = spaces(b, e);
                let assign = match self.c(s + 1) {
                    // `literal-arrow-function`'s parameter
                    Some(b'>') => return None,
                    Some(b'=') => false,
                    // `literal-function`'s or `literal-arrow-function`'s name
                    _ if self.function_value(spaces(b, s + 1)) => return None,
                    _ => true,
                };
                let q = self.variable_at(p, e, upper, var)?;
                // the `=` after blanks the run loop passes: `literal-operators`' assignment
                if q == e
                    && assign
                    && self.straight()
                    && b[e..s].iter().all(|&c| c == b' ' || c == b'\t')
                {
                    return self.tok(s, s + 1, ASSIGN);
                }
                Some(q)
            }
            Some(b'(') => {
                // `literal-method`'s key
                let s = spaces(b, e);
                if self.body_follows(s) {
                    return None;
                }
                self.put(p, e, FUNCTION);
                let c = spaces(b, s + 1);
                if self.is(c, b')') {
                    return self.tok(s, c + 1, CALL_PARENS);
                }
                // the `(`: `brackets`', where no arrow follows
                if self.straight() && !self.arrow_follows(s) && self.can_push() {
                    return self.tok_push(s, s + 1, ROUND_BEGIN, ROUND);
                }
                Some(s)
            }
            Some(b'.' | b'[') if upper => self.upper_member(p, e),
            Some(b'.' | b'[') => {
                if self.is(e, b'.') && has(b, e + 1, b"prototype") {
                    return None;
                }
                let s = spaces(b, e);
                if after_dot {
                    self.put(p, e, PROPERTY);
                    if s != e {
                        return Some(e);
                    }
                } else {
                    self.put(p, s, OBJECT);
                }
                Some(self.after_object(s))
            }
            Some(b':') if core => {
                // the labels' functions and arrows, then `literal-labels`' key
                let s = spaces(b, e);
                if self.function_value(spaces(b, s + 1)) {
                    return None;
                }
                let after_q = before(b, p) == Some(b'?')
                    || space_before(b, p).is_some_and(|s| s > 0 && b[s - 1] == b'?');
                if after_q || after_dot {
                    return self.variable_at(p, e, upper, var);
                }
                self.put(p, e, OBJ_KEY_LABEL);
                self.put(e, s, OBJ_KEY);
                self.tok(s, s + 1, OBJ_KEY_SEP)
            }
            Some(b'`') => None,
            _ => self.variable_at(p, e, upper, var),
        }
    }

    /// `literal-variable` at the name `p..e`: for one starting uppercase or with `_` (`upper`) its
    /// own (a constant, a class's static property), else `var`.
    fn variable_at(&mut self, p: usize, e: usize, upper: bool, var: u16) -> Option<usize> {
        if upper {
            self.variable(p)
        } else {
            self.tok(p, e, var)
        }
    }

    /// [`Scan::plain_name`] for a name `p..e` starting uppercase or with `_` before `.` or `[`:
    /// `literal-method-call`'s class before `.name(`, else `literal-variable`'s, but where a
    /// function or an arrow may be assigned to the member, `prototype` follows, or blanks or no
    /// name come before the member.
    fn upper_member(&mut self, p: usize, e: usize) -> Option<usize> {
        let b = self.b;
        if self.is(spaces(b, e), b'[') {
            return self.variable(p);
        }
        if !self.is(e, b'.') {
            return None;
        }
        let m = ident(b, e + 1)?;
        if &b[e + 1..m] == b"prototype" {
            return None;
        }
        let ms = spaces(b, m);
        match self.c(ms) {
            Some(b'=')
                if self.c(ms + 1) != Some(b'=') && self.function_value(spaces(b, ms + 1)) =>
            {
                None
            }
            Some(b'(')
                if b[p].is_ascii_uppercase() && (before(b, p) == Some(b'.') || boundary(b, p)) =>
            {
                self.put(p, e, CLASS_VAR);
                self.put(e, e + 1, ACCESSOR);
                self.put(e + 1, m, FUNCTION);
                let c = spaces(b, ms + 1);
                if self.is(c, b')') {
                    return self.tok(ms, c + 1, CALL_PARENS);
                }
                Some(ms)
            }
            _ => self.variable(p),
        }
    }

    /// [`Scan::plain_name`] for a word of `literal-keywords`', `literal-language-constant`'s or
    /// `literal-language-variable`'s alone (`mask`): the patterns before theirs take one only
    /// after a `.`, as a function's name before `=`, a label before `:` in `core`, a class
    /// before `.prototype` or, but for a keyword, a method's name before `(…) {`. `None` for
    /// those, and for a word of other patterns.
    fn keyword_name(
        &mut self,
        p: usize,
        e: usize,
        next: Option<u8>,
        mask: u16,
        core: bool,
    ) -> Option<usize> {
        let b = self.b;
        if before(b, p) == Some(b'.')
            || next == Some(b'=')
            || (core && next == Some(b':'))
            || (self.is(e, b'.') && has(b, e + 1, b"prototype"))
        {
            return None;
        }
        match mask {
            W_KEYWORD => self.keywords(p),
            // `default`: `literal-keywords` comes first
            m if m == W_KEYWORD | W_MODULE => self.keywords(p),
            W_MODULE => {
                if next == Some(b'(') && self.body_follows(spaces(b, e)) {
                    return None;
                }
                self.module(p)
            }
            W_CONSTANT | W_LANG_VAR => {
                if next == Some(b'(') && self.body_follows(spaces(b, e)) {
                    return None;
                }
                let q = if mask == W_CONSTANT {
                    self.language_constant(p)
                } else {
                    self.language_variable(p)
                }?;
                // `this.`: the `.` in the same step
                if q == e && next == Some(b'.') {
                    return Some(self.after_object(e));
                }
                Some(q)
            }
            _ => None,
        }
    }

    /// Whether the top context's dispatch at a `(`, `[`, `.` or `=` (not `==`) after a name goes
    /// straight to `expression`'s patterns for it: those of code that only close at another byte.
    fn straight(&self) -> bool {
        matches!(
            self.top(),
            MAIN | CURLY
                | ROUND
                | SQUARE
                | SUBST
                | JSX_EXPR
                | SWITCH_BODY
                | CASE
                | PARAM_DEFAULT
                | FOR_IN
                | GQL_SUBST
                | STYLED_ARGS
        )
    }

    /// After a name at `s`, a `.` or `[`, taken in the same step where the top context's
    /// dispatch would give it [`Scan::straight`]: the `.` before a member that no `=` follows,
    /// `expression`'s accessor, or `brackets`' `[`. Where the scan goes on.
    fn after_object(&mut self, s: usize) -> usize {
        if !self.straight() {
            return s;
        }
        if self.is(s, b'.') {
            if self.plain_dot(s) {
                self.put(s, s + 1, ACCESSOR);
                self.member = s + 1;
                return s + 1;
            }
            return s;
        }
        if self.is(s, b'[') && self.can_push() {
            self.put(s, s + 1, SQUARE_BEGIN);
            self.push(SQUARE);
            return s + 1;
        }
        s
    }

    /// Whether the `.` at `p` before a name can only be `literal-operators`' accessor: no
    /// `prototype` follows, nor a `=` that may assign a function or an arrow to the member
    /// (`literal-function`'s and `literal-arrow-function`'s members), and a number needs a digit.
    fn plain_dot(&self, p: usize) -> bool {
        let b = self.b;
        if !ident_start(self.c(p + 1)) {
            return false;
        }
        let n = self.name_at(p + 1);
        if &b[p + 1..n.end] == b"prototype" {
            return false;
        }
        if n.next != Some(b'=') {
            return true;
        }
        let t = spaces(b, n.end);
        matches!(self.c(t + 1), Some(b'=' | b'>')) || !self.function_value(spaces(b, t + 1))
    }

    /// Whether what starts at `q`, after a name's `=` or `:`, may be a function or an arrow's head:
    /// `async`, `function`, `(…) =>` or a name before `=>`.
    fn function_value(&self, q: usize) -> bool {
        let b = self.b;
        match self.c(q) {
            Some(b'(') => self.arrow_follows(q),
            Some(c) if ident_start(Some(c)) => {
                (c == b'a' && has(b, q, b"async"))
                    || (c == b'f' && has(b, q, b"function"))
                    || ident(b, q).is_some_and(|e| has(b, spaces(b, e), b"=>"))
            }
            _ => false,
        }
    }

    /// Whether a pattern of `core` or `expression` can start at the blank at `p`: one after a
    /// `.` (`(?<=\.)\s*(…)`), or one before `:`, `gql`, `` ` ``, `/` or `<` after the spaces.
    fn blank_may_match(&self, p: usize) -> bool {
        before(self.b, p) == Some(b'.')
            || matches!(
                self.c(self.spaces_at(p)),
                Some(b':' | b'g' | b'`' | b'/' | b'<')
            )
    }

    /// `expression`.
    fn expression(&mut self, p: usize) -> Option<usize> {
        if p >= self.b.len() {
            return None;
        }
        // only the patterns that can start with the byte at `p`, in the grammar's order
        match self.b[p] {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => self.name_patterns(p),
            b'0'..=b'9' => self.method(p).or_else(|| self.number(p)),
            // `.name` that no `=` follows: only the accessor (`function`'s and
            // `literal-arrow-function`'s members want `=`, and a number a digit)
            b'.' if self.plain_dot(p) => self.tok(p, p + 1, ACCESSOR),
            b'.' => self
                .function(p)
                .or_else(|| self.arrow_function(p))
                .or_else(|| self.number(p))
                .or_else(|| self.operators(p)),
            // `literal-method`'s quoted key only before `(`
            b'"' | b'\''
                if !quoted(self.b, p).is_some_and(|e| self.is(spaces(self.b, e), b'(')) =>
            {
                self.string(p)
            }
            b'"' | b'\'' => self.method(p).or_else(|| self.string(p)),
            b'`' => self.template_start(p),
            // `literal-variable`'s `(?<=\.)\s*(…)` starts at a space too
            b' ' | b'\t' | 0x0b | 0x0c | b'\r' if !self.blank_may_match(p) => None,
            b' ' | b'\t' | 0x0b | 0x0c | b'\r' => self
                .regexp(p)
                .or_else(|| self.jsx(p))
                .or_else(|| self.variable(p)),
            // at `(`, `literal-arrow-function` can only be its `(?=(…)\s*(=>))`
            b'(' => {
                if self.arrow_follows(p) {
                    if !self.can_push() {
                        return self.brackets(p);
                    }
                    return self.enter(p, ARROW).or_else(|| self.brackets(p));
                }
                self.brackets(p)
            }
            b'/' => self
                .regexp(p)
                .or_else(|| self.comments(p))
                .or_else(|| self.operators(p)),
            b'<' => self
                .merge_conflict(p)
                .or_else(|| self.jsx(p))
                .or_else(|| self.comments(p))
                .or_else(|| self.operators(p)),
            b'-' => self
                .number(p)
                .or_else(|| self.comments(p))
                .or_else(|| self.operators(p)),
            b'+' => self.number(p).or_else(|| self.operators(p)),
            b'@' => self.decorator(p),
            b'{' => self.support_other(p).or_else(|| self.brackets(p)),
            b'[' => self.brackets(p),
            b'*' => self.method(p).or_else(|| self.operators(p)),
            b'=' | b'|' | b'>' => self.merge_conflict(p).or_else(|| self.operators(p)),
            b'#' => self.comments(p),
            b'!' | b'%' | b'&' | b'^' | b'~' | b'?' | b':' => self.operators(p),
            0x80.. if !self.wide_space_or_digit(p) => None,
            0x80.. => self.expression_all(p),
            _ => None,
        }
    }

    /// `expression` at a name: the name read once, and only the patterns that can take it
    /// tried, in the grammar's order.
    fn name_patterns(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.plain_name(p, false) {
            return Some(q);
        }
        let b = self.b;
        let Name {
            end: e,
            next,
            word_end: we,
            mask,
        } = self.name_at(p);
        let at_e = self.c(e);
        let w = &b[p..we];
        let after_dot = before(b, p) == Some(b'.');
        let upper = b[p].is_ascii_uppercase() || b[p] == b'_';
        // `(\b_?[A-Z][$\w]*)?(\.)…`: a class's name before its member
        let class_member = upper && at_e == Some(b'.');
        let async_or_fn = matches!(w, b"async" | b"function");
        // each word pattern is tried only for a word it can take ([`word_mask`])
        let can = |bit: u16| mask & bit != 0;
        if can(W_SUPPORT_CLASS)
            && let Some(q) = self.support_class(p)
        {
            return Some(q);
        }
        if can(W_SUPPORT_OTHER)
            && let Some(q) = self.support_other(p)
        {
            return Some(q);
        }
        if (next == Some(b'=') || async_or_fn || class_member)
            && let Some(q) = self.function(p).or_else(|| self.arrow_function(p))
        {
            return Some(q);
        }
        if at_e == Some(b'.')
            && let Some(q) = self.prototype(p)
        {
            return Some(q);
        }
        if can(W_KEYWORD)
            && let Some(q) = self.keywords(p)
        {
            return Some(q);
        }
        if (next == Some(b'(') || matches!(w, b"static" | b"async" | b"get" | b"set"))
            && let Some(q) = self.method(p)
        {
            return Some(q);
        }
        if mask & (W_MODULE | W_CLASS | W_FLOW | W_NUMBER | W_CONSTANT | W_LANG_VAR | W_NEW) != 0 {
            let r = if can(W_MODULE) { self.module(p) } else { None }
                .or_else(|| if can(W_CLASS) { self.class(p) } else { None })
                .or_else(|| {
                    if can(W_FLOW) {
                        self.flow_declaration(p)
                    } else {
                        None
                    }
                })
                .or_else(|| if can(W_NUMBER) { self.number(p) } else { None })
                .or_else(|| {
                    if can(W_CONSTANT) {
                        self.language_constant(p)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    if can(W_LANG_VAR) {
                        self.language_variable(p)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    if can(W_NEW) {
                        self.constructor(p)
                    } else {
                        None
                    }
                });
            if r.is_some() {
                return r;
            }
        }
        if (after_dot || upper || next == Some(b'(') || next == Some(b'.'))
            && let Some(q) = self.method_call(p)
        {
            return Some(q);
        }
        if matches!(next, Some(b'(' | b'`'))
            && let Some(q) = self.function_call(p)
        {
            return Some(q);
        }
        if can(W_OPERATOR)
            && let Some(q) = self.operators(p)
        {
            return Some(q);
        }
        self.variable(p)
    }

    /// Whether the character at `p`, past ASCII, is `\s` or `\d`: where a pattern of `core` or
    /// `expression` can start at one (a key's digits, `literal-labels`' `\s*:`, `literal-regexp`'s
    /// and `literal-jsx`'s spaces, `literal-variable`'s after a `.`).
    #[cold]
    fn wide_space_or_digit(&self, p: usize) -> bool {
        wide_space(self.b, p).is_some() || wide_digit(self.b, p).is_some()
    }

    /// `expression` with every pattern tried, for a byte past ASCII.
    #[cold]
    fn expression_all(&mut self, p: usize) -> Option<usize> {
        self.merge_conflict(p)
            .or_else(|| self.regexp(p))
            .or_else(|| self.jsx(p))
            .or_else(|| self.decorator(p))
            .or_else(|| self.support_class(p))
            .or_else(|| self.support_other(p))
            .or_else(|| self.function(p))
            .or_else(|| self.arrow_function(p))
            .or_else(|| self.prototype(p))
            .or_else(|| self.keywords(p))
            .or_else(|| self.method(p))
            .or_else(|| self.module(p))
            .or_else(|| self.class(p))
            .or_else(|| self.flow_declaration(p))
            .or_else(|| self.number(p))
            .or_else(|| self.template_start(p))
            .or_else(|| self.string(p))
            .or_else(|| self.language_constant(p))
            .or_else(|| self.language_variable(p))
            .or_else(|| self.constructor(p))
            .or_else(|| self.method_call(p))
            .or_else(|| self.function_call(p))
            .or_else(|| self.comments(p))
            .or_else(|| self.brackets(p))
            .or_else(|| self.operators(p))
            .or_else(|| self.variable(p))
    }

    /// A key's captures: a quoted one's quotes and text, or a name.
    fn key_caps(&mut self, p: usize, k: Key) {
        if k.quote.is_some() {
            self.put(p, p + 1, STR_BEGIN);
            self.put(p + 1, k.end - 1, KEY_FN);
            self.put(k.end - 1, k.end, STR_END);
        } else {
            self.put(p, k.end, KEY_NAME_FN);
        }
    }

    /// `\b(async)\s+` at `q` (or without the `\b`): where its spaces end.
    fn async_at(&self, q: usize, bound: bool) -> Option<usize> {
        if !has(self.b, q, b"async") || (bound && !boundary(self.b, q)) {
            return None;
        }
        space_len(self.b, q + 5)?;
        Some(spaces(self.b, q + 5))
    }

    /// `(function)(?>\s*(\*)|(?=[\s(<]))\s*([_$a-zA-Z][$\w]*)?` at `q`: the asterisk, the name
    /// and where it ends.
    fn function_tail(&self, q: usize) -> Option<FunctionTail> {
        if !has(self.b, q, b"function") {
            return None;
        }
        let f = q + 8;
        let s = spaces(self.b, f);
        let (star, mut e) = if self.is(s, b'*') {
            (Some(s), s + 1)
        } else if space_len(self.b, f).is_some() || matches!(self.c(f), Some(b'(' | b'<')) {
            (None, f)
        } else {
            return None;
        };
        e = spaces(self.b, e);
        let name = ident(self.b, e).map(|n| (e, n));
        if let Some((_, n)) = name {
            e = n;
        }
        Some((star, name, e))
    }

    /// `literal-function-labels`.
    fn function_labels(&mut self, p: usize) -> Option<usize> {
        let k = key(self.b, p)?;
        let colon = spaces(self.b, k.end);
        if !self.is(colon, b':') {
            return None;
        }
        let q = spaces(self.b, colon + 1);
        let (asy, (star, name, e)) = match self.async_at(q, true) {
            Some(r) if let Some(t) = self.function_tail(r) => (Some(q), t),
            _ => (None, self.function_tail(q)?),
        };
        if !self.can_push() {
            return None;
        }
        let f = if asy.is_some() {
            spaces(self.b, q + 5)
        } else {
            q
        };
        self.fill(p);
        self.key_caps(p, k);
        self.put(colon, colon + 1, KEY_SEP);
        if let Some(a) = asy {
            self.put(a, a + 5, STORAGE);
        }
        self.put(f, f + 8, STORAGE_FN);
        if let Some(s) = star {
            self.put(s, s + 1, ASTERISK);
        }
        if let Some((s, n)) = name {
            self.put(s, n, ENTITY_FN);
        }
        self.put(e, e, PLAIN);
        self.push(FUNC);
        Some(e)
    }

    /// `literal-arrow-function-labels`.
    fn arrow_labels(&mut self, p: usize) -> Option<usize> {
        let k = key(self.b, p)?;
        let colon = spaces(self.b, k.end);
        if !self.is(colon, b':') {
            return None;
        }
        let q = spaces(self.b, colon + 1);
        // `\s*(?:\b(async)\s+)?\s*(?=(…)\s*(=>))`, pushing
        let lookahead = match self.async_at(q, true) {
            Some(r) if self.arrow_follows(r) => Some((Some(q), r)),
            _ => self.arrow_follows(q).then_some((None, q)),
        };
        if let Some((asy, e)) = lookahead {
            if !self.can_push() {
                return None;
            }
            self.fill(p);
            self.key_caps(p, k);
            self.put(colon, colon + 1, KEY_SEP);
            if let Some(a) = asy {
                self.put(a, a + 5, STORAGE);
            }
            self.put(e, e, PLAIN);
            self.push(ARROW);
            return Some(e);
        }
        // `\s*(?:\b(async)\s+)?\s*\b([_$a-zA-Z][$\w]*)\s*(=>)`
        let param = |r: usize| {
            if !boundary(self.b, r) {
                return None;
            }
            let e = ident(self.b, r)?;
            let s = spaces(self.b, e);
            has(self.b, s, b"=>").then_some((r, e, s))
        };
        let (asy, (r, e, s)) = match self.async_at(q, true) {
            Some(r) if let Some(m) = param(r) => (Some(q), m),
            _ => (None, param(q)?),
        };
        self.key_caps(p, k);
        self.put(colon, colon + 1, KEY_SEP);
        if let Some(a) = asy {
            self.put(a, a + 5, STORAGE);
        }
        self.put(r, e, PARAM);
        self.tok(s, s + 2, ARROW_KW)
    }

    /// `literal-labels`.
    fn labels(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let after_q = before(b, p) == Some(b'?');
        let after_q_space = space_before(b, p).is_some_and(|s| s > 0 && b[s - 1] == b'?');
        if !after_q && !after_q_space {
            // `(?=(?>'…'|"…"|)\s*:)`, pushing nothing
            let colon = match self.c(p) {
                Some(b'\'' | b'"') => quoted(b, p).map(|e| spaces(b, e)),
                _ => Some(self.spaces_at(p)),
            };
            if colon.is_some_and(|c| self.is(c, b':')) {
                return self.enter(p, LABEL);
            }
        }
        // `(?<!\.|\?|\?\s)([_$a-zA-Z][$\w]*)\s*(:)`
        if after_q || after_q_space || !self.not_after_dot(p) {
            return None;
        }
        let e = ident(b, p)?;
        let s = spaces(b, e);
        if !self.is(s, b':') {
            return None;
        }
        self.put(p, e, OBJ_KEY_LABEL);
        self.put(e, s, OBJ_KEY);
        self.tok(s, s + 1, OBJ_KEY_SEP)
    }

    /// `literal-for` and `literal-switch`.
    fn for_switch(&mut self, p: usize) -> Option<usize> {
        if !self.not_after_dot(p) {
            return None;
        }
        match self.word(p)? {
            b"for" => self.tok_push(p, p + 3, LOOP, FOR),
            b"switch" => self.tok_push(p, p + 6, SWITCH_KW, SWITCH),
            _ => None,
        }
    }

    /// `styled-components`.
    fn styled(&mut self, p: usize) -> Option<usize> {
        if !self.not_after_dot(p) || !boundary(self.b, p) {
            return None;
        }
        let b = self.b;
        for name in [&b"injectGlobal"[..], b"keyframes"] {
            if has(b, p, name) {
                let s = spaces(b, p + name.len());
                if self.is(s, b'`') && self.can_push() {
                    self.put(p, p + name.len(), TAGGED);
                    self.put(s, s + 1, TPL_MARK_BEGIN);
                    self.push(STYLED);
                    return Some(s + 1);
                }
            }
        }
        if !has(b, p, b"styled") {
            return None;
        }
        let q = p + 6;
        if self.is(q, b'.')
            && let Some(e) = ident(b, q + 1)
        {
            let s = spaces(b, e);
            if self.is(s, b'`') && self.can_push() {
                self.put(p, q, OBJECT);
                self.put(q, q + 1, PUNCT_ACCESSOR);
                self.put(q + 1, e, TAGGED);
                self.put(s, s + 1, TPL_MARK_BEGIN);
                self.push(STYLED);
                return Some(s + 1);
            }
        }
        let s = spaces(b, q);
        let tpl = self
            .balanced(s)
            .is_some_and(|e| self.is(spaces(b, e), b'`'));
        if tpl && self.can_push() {
            self.put(p, q, STYLED_FN);
            self.put(q, s, PLAIN);
            self.push(STYLED_CALL);
            return Some(s);
        }
        None
    }

    /// `graphql`: `\s*+gql`` or `\s*+`#graphql`.
    fn graphql(&mut self, p: usize) -> Option<usize> {
        let s = self.spaces_at(p);
        let e = if has(self.b, s, b"gql`") {
            s + 4
        } else if has(self.b, s, b"`#graphql") {
            s + 9
        } else {
            return None;
        };
        self.tok_push(p, e, PLAIN, GRAPHQL)
    }

    /// `merge-conflits`.
    fn merge_conflict(&mut self, p: usize) -> Option<usize> {
        if p != 0 || self.b.len() < 7 {
            return None;
        }
        let b = self.b;
        let run = |c: u8| b[..7].iter().all(|&x| x == c);
        if run(b'<') || run(b'>') {
            // `\s(.+)$`
            let k = space_len(b, 7)?;
            if 7 + k >= self.b.len() {
                return None;
            }
            let leaf = if b[0] == b'<' {
                CONFLICT_INTO
            } else {
                CONFLICT_OTHER
            };
            self.put(0, 7, leaf);
            return self.tok(7 + k, self.b.len(), CONFLICT_TEXT);
        }
        if self.b.len() == 7 && b[..7].iter().all(|&x| x == b'=' || x == b'|') {
            return self.tok(0, 7, CONFLICT_SEP);
        }
        None
    }

    /// `literal-regexp`.
    fn regexp(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let s = self.spaces_at(p);
        if !self.is(s, b'/') || matches!(self.c(s + 1), Some(b'/' | b'*')) || eol(b, s + 1) {
            return None;
        }
        let after = p == 0
            || matches!(
                before(b, p).unwrap_or(b'\n'),
                b'.' | b'('
                    | b','
                    | b'{'
                    | b'}'
                    | b'['
                    | b';'
                    | b'<'
                    | b'>'
                    | b'='
                    | b'!'
                    | b'+'
                    | b'-'
                    | b'*'
                    | b'%'
                    | b'&'
                    | b'|'
                    | b'^'
                    | b'~'
                    | b'?'
                    | b':'
                    | b'/'
            )
            || BEFORE_REGEX.iter().any(|w| self.after_word(p, w));
        if !after || !self.can_push() {
            return None;
        }
        self.put(p, s, RE);
        self.put(s, s + 1, RE_BEGIN);
        self.push(REGEX);
        Some(s + 1)
    }

    /// `(?<=\Wword|^word)` at `p`.
    fn after_word(&self, p: usize, w: &[u8]) -> bool {
        if p < w.len() || p > self.b.len() || &self.b[p - w.len()..p] != w {
            return false;
        }
        let s = p - w.len();
        s == 0 || !is_word(self.b, s - 1)
    }

    /// `literal-jsx`.
    fn jsx(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let s = self.spaces_at(p);
        if !self.is(s, b'<') || !ident_start(self.c(s + 1)) {
            return None;
        }
        let after = p == 0
            || matches!(
                before(b, p),
                Some(b'(' | b'{' | b'[' | b',' | b'?' | b':' | b'=')
            )
            || (p >= 2 && p <= self.b.len() && matches!(&b[p - 2..p], b"&&" | b"||" | b"=>"))
            || self.after_word(p, b"return")
            || self.after_word(p, b"default");
        if !after {
            return None;
        }
        self.enter(p, JSX)
    }

    /// `es7-decorators`: `(@)([_$a-zA-Z][$\w]*)\b`.
    fn decorator(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'@') {
            return None;
        }
        let mut e = ident(self.b, p + 1)?;
        // the name backs off to a `\b`
        while !boundary(self.b, e) {
            if e <= p + 2 {
                return None;
            }
            e -= 1;
            while self.b[e] & 0xc0 == 0x80 {
                e -= 1;
            }
        }
        self.put(p, p + 1, DEC_MARK);
        self.tok(p + 1, e, DEC_NAME)
    }

    /// `support-class`.
    fn support_class(&mut self, p: usize) -> Option<usize> {
        let w = self.word(p)?;
        // the lists' words only ([`word_mask`]), before searching them
        if word_mask(w) & W_SUPPORT_CLASS == 0 {
            return None;
        }
        let e = p + w.len();
        if self.not_after_dot(p) {
            if BUILTIN_CLASSES.contains(&w) {
                return self.tok(p, e, BUILTIN_CLASS);
            }
            if ERRORS.contains(&w) {
                return self.tok(p, e, ERROR_CLASS);
            }
        }
        (w == b"Buffer")
            .then(|| self.tok(p, e, NODE_CLASS))
            .flatten()
    }

    /// `support-other`.
    fn support_other(&mut self, p: usize) -> Option<usize> {
        if has(self.b, p, b"{{") {
            return self.tok_push(p, p + 2, PLAIN, MUSTACHE);
        }
        if !self.not_after_dot(p) {
            return None;
        }
        let w = self.word(p)?;
        if word_mask(w) & W_SUPPORT_OTHER == 0 {
            return None;
        }
        let e = p + w.len();
        let leaf = if BUILTIN_FNS.contains(&w) {
            BUILTIN_FN
        } else if NODE_FNS.contains(&w) {
            NODE_FN
        } else if w == b"document" || w == b"window" {
            DOM
        } else if matches!(
            w,
            b"global" | b"GLOBAL" | b"root" | b"__dirname" | b"__filename"
        ) {
            NODE_OBJ
        } else {
            return self.support_object(p, w);
        };
        self.tok(p, e, leaf)
    }

    /// `console`, `process`, `module` and `exports`, with a member of theirs.
    fn support_object(&mut self, p: usize, w: &[u8]) -> Option<usize> {
        let e = p + w.len();
        let (leaf, members): (u16, Members) = match w {
            b"console" => (CONSOLE, &[(CONSOLE_FNS, CONSOLE_FN)]),
            b"process" => (
                PROCESS,
                &[(PROCESS_PROPS, PROCESS), (PROCESS_FNS, PROCESS_FN)],
            ),
            b"exports" | b"module" => (MODULE_OBJ, &[(MODULE_PROPS, MODULE_OBJ)]),
            _ => return None,
        };
        self.put(p, e, leaf);
        if self.is(e, b'.')
            && let Some(m) = self.word(e + 1)
        {
            let m_end = e + 1 + m.len();
            for &(set, mleaf) in members {
                if set.contains(&m) {
                    self.put(e, e + 1, ACCESSOR);
                    return self.tok(e + 1, m_end, mleaf);
                }
            }
        }
        Some(e)
    }

    /// `literal-function`.
    fn function(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // `(?:([_$a-zA-Z][$\w]*)\s*(=)\s*)?(?:(async)\s+)?(function)…`
        let prefix = ident(b, p).and_then(|e| {
            let s = spaces(b, e);
            self.is(s, b'=').then(|| (e, s, spaces(b, s + 1)))
        });
        let mut found = None;
        for (pre, q) in [(prefix, prefix.map(|x| x.2)), (None, Some(p))] {
            let Some(q) = q else { continue };
            if let Some(r) = self.async_at(q, false)
                && let Some(t) = self.function_tail(r)
            {
                found = Some((pre, Some(q), r, t));
                break;
            }
            if let Some(t) = self.function_tail(q) {
                found = Some((pre, None, q, t));
                break;
            }
        }
        if let Some((pre, asy, f, (star, name, e))) = found {
            if !self.can_push() {
                return None;
            }
            if let Some((ne, s, _)) = pre {
                self.put(p, ne, ENTITY_FN);
                self.put(s, s + 1, ASSIGN);
            }
            if let Some(a) = asy {
                self.put(a, a + 5, STORAGE);
            }
            self.put(f, f + 8, STORAGE_FN);
            if let Some(s) = star {
                self.put(s, s + 1, ASTERISK);
            }
            if let Some((s, n)) = name {
                self.put(s, n, ENTITY_FN);
            }
            self.put(e, e, PLAIN);
            self.push(FUNC);
            return Some(e);
        }
        // `(\b_?[A-Z][$\w]*)?(\.)(prototype)(\.)…` and `(\b_?[A-Z][$\w]*)?(\.)(…)\s*(=)…`
        let (class, dot) = self.class_dot(p)?;
        let (proto, name) = self.member_after(dot)?;
        let s = spaces(b, name.1);
        if !self.is(s, b'=') {
            return None;
        }
        let q = spaces(b, s + 1);
        let (asy, f, (star, fname, e)) = match self.async_at(q, false) {
            Some(r) if let Some(t) = self.function_tail(spaces(b, r)) => (Some(q), spaces(b, r), t),
            _ => (None, q, self.function_tail(q)?),
        };
        let e = spaces(b, e);
        if !self.can_push() {
            return None;
        }
        if let Some(c) = class {
            self.put(p, c, ENTITY_CLASS);
        }
        self.put(dot, dot + 1, ACCESSOR);
        if let Some(pe) = proto {
            self.put(dot + 1, pe, PROTOTYPE);
            self.put(pe, pe + 1, ACCESSOR);
        }
        self.put(name.0, name.1, ENTITY_FN);
        self.put(s, s + 1, ASSIGN);
        if let Some(a) = asy {
            self.put(a, a + 5, STORAGE);
        }
        self.put(f, f + 8, STORAGE_FN);
        if let Some(st) = star {
            self.put(st, st + 1, ASTERISK);
        }
        if let Some((st, n)) = fname {
            self.put(st, n, ENTITY_FN);
        }
        self.put(e, e, PLAIN);
        self.push(FUNC);
        Some(e)
    }

    /// `(\b_?[A-Z][$\w]*)?(\.)` at `p`: where the class's name ends, and the `.`.
    fn class_dot(&self, p: usize) -> Option<(Option<usize>, usize)> {
        let b = self.b;
        if boundary(b, p) {
            let u = if self.is(p, b'_') { p + 1 } else { p };
            if self.c(u).is_some_and(|c| c.is_ascii_uppercase()) {
                let e = name_end(b, u + 1);
                if self.is(e, b'.') {
                    return Some((Some(e), e));
                }
            }
        }
        self.is(p, b'.').then_some((None, p))
    }

    /// After the `.` at `dot`: `(prototype)(\.)([_$a-zA-Z][$\w]*)` (the `prototype`'s end, then
    /// the name), else `([_$a-zA-Z][$\w]*)`.
    #[allow(clippy::type_complexity)]
    fn member_after(&self, dot: usize) -> Option<(Option<usize>, (usize, usize))> {
        let b = self.b;
        if has(b, dot + 1, b"prototype")
            && self.is(dot + 10, b'.')
            && let Some(e) = ident(b, dot + 11)
        {
            return Some((Some(dot + 10), (dot + 11, e)));
        }
        let e = ident(b, dot + 1)?;
        Some((None, (dot + 1, e)))
    }

    /// `literal-arrow-function`.
    fn arrow_function(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let prefix = ident(b, p).and_then(|e| {
            let s = spaces(b, e);
            self.is(s, b'=').then(|| (e, s, spaces(b, s + 1)))
        });
        let starts = [(prefix, prefix.map(|x| x.2)), (None, Some(p))];
        // `(?:…)?(?:\b(async)\s+)?(?=(…)\s*(=>))`, pushing
        for &(pre, q) in &starts {
            let Some(q) = q else { continue };
            let found = match self.async_at(q, true) {
                Some(r) if self.arrow_follows(r) => Some((Some(q), r)),
                _ => self.arrow_follows(q).then_some((None, q)),
            };
            if let Some((asy, e)) = found {
                if !self.can_push() {
                    return None;
                }
                if e == p {
                    return self.enter(p, ARROW);
                }
                if let Some((ne, s, _)) = pre {
                    self.put(p, ne, ENTITY_FN);
                    self.put(s, s + 1, ASSIGN);
                }
                if let Some(a) = asy {
                    self.put(a, a + 5, STORAGE);
                }
                self.put(e, e, PLAIN);
                self.push(ARROW);
                return Some(e);
            }
        }
        // `(?:…)?(?:(async)\s+)?\b([_$a-zA-Z][$\w]*)\s*(=>)`
        let param = |r: usize| {
            if !boundary(b, r) {
                return None;
            }
            let e = ident(b, r)?;
            let s = spaces(b, e);
            has(b, s, b"=>").then_some((r, e, s))
        };
        for &(pre, q) in &starts {
            let Some(q) = q else { continue };
            let found = match self.async_at(q, false) {
                Some(r) if let Some(m) = param(r) => Some((Some(q), m)),
                _ => param(q).map(|m| (None, m)),
            };
            if let Some((asy, (r, e, s))) = found {
                if let Some((ne, es, _)) = pre {
                    self.put(p, ne, ENTITY_FN);
                    self.put(es, es + 1, ASSIGN);
                }
                if let Some(a) = asy {
                    self.put(a, a + 5, STORAGE);
                }
                self.put(r, e, PARAM);
                return self.tok(s, s + 2, ARROW_KW);
            }
        }
        // the four with a class's member, `…(\.)(prototype)(\.)(…)`'s two first: each pushes
        // before `(…) =>`, else takes a parameter and its `=>`
        let (class, dot) = self.class_dot(p)?;
        // `prototype.name` first, then the name after the `.`
        let proto_member = (has(b, dot + 1, b"prototype") && self.is(dot + 10, b'.'))
            .then(|| ident(b, dot + 11).map(|e| (Some(dot + 10), (dot + 11, e))))
            .flatten();
        let member = ident(b, dot + 1).map(|e| (None, (dot + 1, e)));
        for (proto, name) in [proto_member, member].into_iter().flatten() {
            let s = spaces(b, name.1);
            if !self.is(s, b'=') {
                continue;
            }
            let q = spaces(b, s + 1);
            // `(async)?` takes `async` when it is there, and gives it back if what follows fails
            let asy = has(b, q, b"async").then_some(q);
            let after = |a: Option<usize>| spaces(b, a.map_or(q, |a| a + 5));
            let options = [asy, None];
            let tries = if asy.is_some() {
                &options[..]
            } else {
                &options[1..]
            };
            // `(async)?\s*(?=(…)\s*(=>))`
            let push = tries
                .iter()
                .find_map(|&a| self.arrow_follows(after(a)).then(|| (a, after(a))));
            // `(async)?\s*\b([_$a-zA-Z][$\w]*)\s*(=>)`
            let tok = push.is_none().then(|| {
                tries.iter().find_map(|&a| {
                    let r = after(a);
                    if !boundary(b, r) {
                        return None;
                    }
                    let e = ident(b, r)?;
                    let t = spaces(b, e);
                    has(b, t, b"=>").then_some((a, r, e, t))
                })
            });
            let tok = tok.flatten();
            if push.is_none() && tok.is_none() {
                continue;
            }
            if push.is_some() && !self.can_push() {
                return None;
            }
            if let Some(c) = class {
                self.put(p, c, ENTITY_CLASS);
            }
            self.put(dot, dot + 1, ACCESSOR);
            if let Some(pe) = proto {
                self.put(dot + 1, pe, PROTOTYPE);
                self.put(pe, pe + 1, ACCESSOR);
            }
            self.put(name.0, name.1, ENTITY_FN);
            self.put(s, s + 1, ASSIGN);
            if let Some((a, e)) = push {
                if let Some(a) = a {
                    self.put(a, a + 5, STORAGE);
                }
                self.put(e, e, PLAIN);
                self.push(ARROW);
                return Some(e);
            }
            let (a, r, e, t) = tok?;
            if let Some(a) = a {
                self.put(a, a + 5, STORAGE);
            }
            self.put(r, e, PARAM);
            return self.tok(t, t + 2, ARROW_KW);
        }
        None
    }

    /// `literal-prototype`.
    fn prototype(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let e = ident(b, p)?;
        if !self.is(e, b'.') || !has(b, e + 1, b"prototype") {
            return None;
        }
        let pe = e + 10;
        let s = spaces(b, pe);
        if self.is(s, b'=') {
            self.put(p, e, ENTITY_CLASS);
            self.put(e, e + 1, ACCESSOR);
            self.put(e + 1, pe, PROTOTYPE);
            self.put(s, s + 1, ASSIGN);
            return Some(spaces(b, s + 1));
        }
        if !boundary(b, pe) {
            return None;
        }
        self.put(p, e, ENTITY_CLASS);
        self.put(e, e + 1, ACCESSOR);
        self.tok(e + 1, pe, PROTOTYPE)
    }

    /// `literal-keyword-storage`.
    fn storage(&mut self, p: usize) -> Option<usize> {
        let e = self.keyword(p, &[b"const", b"let", b"var"])?;
        self.tok(p, e, STORAGE)
    }

    /// `literal-keywords`.
    fn keywords(&mut self, p: usize) -> Option<usize> {
        if !self.not_after_dot(p) {
            return None;
        }
        let w = self.word(p)?;
        let e = p + w.len();
        let leaf = match w {
            b"const" | b"let" | b"var" => STORAGE,
            b"await" | b"yield" => {
                self.put(p, e, FLOW_KW);
                let s = spaces(self.b, e);
                if self.is(s, b'*') {
                    return self.tok(s, s + 1, ASTERISK);
                }
                return Some(e);
            }
            b"return" => FLOW_KW,
            b"if" | b"else" => COND,
            b"catch" | b"finally" | b"throw" | b"try" => TRY,
            b"break" | b"continue" | b"do" | b"goto" | b"while" | b"case" | b"default" => LOOP,
            b"enum" | b"module" | b"public" | b"package" | b"private" | b"interface"
            | b"protected" => RESERVED,
            b"debugger" => KW_OTHER,
            _ => return None,
        };
        self.tok(p, e, leaf)
    }

    /// `literal-method`.
    fn method(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // without `static`, `async`, `*`, `get` or `set` at `p`, only the bare key can match
        if !matches!(self.c(p), Some(b's' | b'a' | b'*' | b'g')) {
            let key = key(b, p)?;
            if !self.body_follows(spaces(b, key.end)) || !self.can_push() {
                return None;
            }
            self.key_caps(p, key);
            self.push(METHOD);
            return Some(key.end);
        }
        // `(?:\b(static)\s+)?(?:\b(async)\s+)?(?:(\*)\s*)?(?>KEY)(?=\s*(…)(?>\s|/\*.*\*/)*\{)`
        let word_spaces = |q: usize, w: &[u8]| {
            if !has(b, q, w) || !boundary(b, q) {
                return None;
            }
            space_len(b, q + w.len())?;
            Some(spaces(b, q + w.len()))
        };
        let statics = [
            word_spaces(p, b"static").map(|e| (Some(p), e)),
            Some((None, p)),
        ];
        for (st, q) in statics.into_iter().flatten() {
            let asyncs = [
                word_spaces(q, b"async").map(|e| (Some(q), e)),
                Some((None, q)),
            ];
            for (asy, r) in asyncs.into_iter().flatten() {
                let stars = [
                    self.is(r, b'*').then(|| (Some(r), spaces(b, r + 1))),
                    Some((None, r)),
                ];
                for (star, k) in stars.into_iter().flatten() {
                    let Some(key) = key(b, k) else { continue };
                    if !self.body_follows(spaces(b, key.end)) {
                        continue;
                    }
                    if !self.can_push() {
                        return None;
                    }
                    if let Some(s) = st {
                        self.put(s, s + 6, STORAGE);
                    }
                    if let Some(a) = asy {
                        self.put(a, a + 5, STORAGE);
                    }
                    if let Some(s) = star {
                        self.put(s, s + 1, ASTERISK);
                    }
                    self.key_caps(k, key);
                    self.push(METHOD);
                    return Some(key.end);
                }
            }
        }
        // `\b(?:(static)\s+)?(get|set)\s+([_$a-zA-Z][$\w]*|\d+)\s*(?=(…)(?>…)*\{)`
        if !boundary(b, p) {
            return None;
        }
        let statics = [
            word_spaces(p, b"static").map(|e| (Some(p), e)),
            Some((None, p)),
        ];
        for (st, q) in statics.into_iter().flatten() {
            if !(has(b, q, b"get") || has(b, q, b"set")) || space_len(b, q + 3).is_none() {
                continue;
            }
            let r = spaces(b, q + 3);
            let Some(e) = ident(b, r).or_else(|| digits(b, r)) else {
                continue;
            };
            let s = spaces(b, e);
            if !self.body_follows(s) {
                continue;
            }
            if !self.can_push() {
                return None;
            }
            if let Some(st) = st {
                self.put(st, st + 6, STORAGE);
            }
            self.put(q, q + 3, ACCESSOR_KW);
            self.put(r, e, ENTITY_ACCESSOR);
            self.put(s, s, PLAIN);
            self.push(METHOD);
            return Some(s);
        }
        None
    }

    /// `literal-module`.
    fn module(&mut self, p: usize) -> Option<usize> {
        let e = self.keyword(p, &[b"import", b"export", b"default", b"from", b"as"])?;
        self.tok(p, e, MODULE)
    }

    /// `literal-class`.
    fn class(&mut self, p: usize) -> Option<usize> {
        if !self.not_after_dot(p) {
            return None;
        }
        let leaf = match self.word(p)? {
            b"class" => CLASS_KW,
            b"interface" => INTERFACE_KW,
            _ => return None,
        };
        let e = word_end(self.b, p);
        self.tok_push(p, e, leaf, CLASS)
    }

    /// `flowtype-declaration`.
    fn flow_declaration(&mut self, p: usize) -> Option<usize> {
        if !self.not_after_dot(p) {
            return None;
        }
        match self.word(p)? {
            b"declare" => self.tok(p, p + 7, DECLARE),
            b"type" => {
                let s = spaces(self.b, p + 4);
                if ident_start(self.c(s)) {
                    return self.tok_push(p, p + 4, TYPE_KW, FLOW_DECL);
                }
                if self.is(s, b'{') {
                    return self.tok_push(p, p + 4, TYPE_KW, FLOW_DECL_BRACES);
                }
                None
            }
            _ => None,
        }
    }

    /// `literal-number`.
    fn number(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let c = self.c(p)?;
        if !(c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'I')) {
            return None;
        }
        // `\B[-+]`: a sign after a non-word character, or the line's start
        let sign = matches!(c, b'-' | b'+') && !boundary(b, p);
        // `(?:\B[-+]|\b)`: where the number's digits start
        let q = if sign {
            p + 1
        } else if boundary(b, p) {
            p
        } else {
            usize::MAX
        };
        let lower = |i: usize| self.c(i).map(|c| c.to_ascii_lowercase());
        // `(\B|\b[0-9]+)` after a `.` at `d`
        let fraction = |d: usize| {
            if boundary(b, d + 1) {
                ascii_digits(b, d + 1)
            } else {
                Some(d + 1)
            }
        };
        if q != usize::MAX && self.is(q, b'0') {
            // `(?i)…0x[0-9a-f]*\.(\B|\b[0-9]+)`
            if lower(q + 1) == Some(b'x') {
                let d = q
                    + 2
                    + b[q + 2..]
                        .iter()
                        .take_while(|c| c.is_ascii_hexdigit())
                        .count();
                if self.is(d, b'.')
                    && let Some(e) = fraction(d)
                {
                    return self.tok(p, e, INVALID_HEX);
                }
            }
            // `…0[0-9]+\.(\B|\b[0-9]+)`
            if let Some(d) = ascii_digits(b, q + 1)
                && self.is(d, b'.')
                && let Some(e) = fraction(d)
            {
                return self.tok(p, e, INVALID_OCT);
            }
        }
        // `(?xi)(?:\B[-+])?(?:\b0b[0-1]*|\b0o[0-7]*|\b0x[0-9a-f]*|
        //   (\B\.[0-9]+|\b[0-9]+(\.[0-9]*)?)(e[-+]?[0-9]+)?)`
        let body = |q: usize| -> Option<usize> {
            let run = |from: usize, ok: fn(&u8) -> bool| {
                from + b
                    .get(from..)
                    .map_or(0, |r| r.iter().take_while(|c| ok(c)).count())
            };
            if boundary(b, q) && self.is(q, b'0') {
                match lower(q + 1) {
                    Some(b'b') => return Some(run(q + 2, |c| matches!(c, b'0' | b'1'))),
                    Some(b'o') => return Some(run(q + 2, |c| matches!(c, b'0'..=b'7'))),
                    Some(b'x') => return Some(run(q + 2, u8::is_ascii_hexdigit)),
                    _ => {}
                }
            }
            let m = if self.is(q, b'.') && !boundary(b, q) {
                ascii_digits(b, q + 1)?
            } else if boundary(b, q) {
                let d = ascii_digits(b, q)?;
                if self.is(d, b'.') {
                    run(d + 1, u8::is_ascii_digit)
                } else {
                    d
                }
            } else {
                return None;
            };
            if lower(m) == Some(b'e') {
                let s = if matches!(self.c(m + 1), Some(b'-' | b'+')) {
                    m + 2
                } else {
                    m + 1
                };
                if let Some(e) = ascii_digits(b, s) {
                    return Some(e);
                }
            }
            Some(m)
        };
        if sign && let Some(e) = body(p + 1) {
            return self.tok(p, e, NUMBER);
        }
        if let Some(e) = body(p) {
            return self.tok(p, e, NUMBER);
        }
        // `(?:\B[-+]|\b)(Infinity)\b`, the sign in its scope
        if q != usize::MAX && has(b, q, b"Infinity") && boundary(b, q + 8) {
            return self.tok(p, q + 8, INFINITY);
        }
        None
    }

    /// `literal-template-string`'s `` ` ``.
    fn template_start(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'`') {
            return None;
        }
        let e = self.tok_push(p, p + 1, TPL_BEGIN, TPL)?;
        if after_paren(self.b, p) && self.regexp_arg(p) && self.can_push() {
            self.push(RE_STR);
        }
        Some(e)
    }

    /// Whether the string or template opening at `p` is the first argument of `RegExp(`
    /// (`new RegExp(…)` too): a regex, in ECMAScript's dialect, as WebStorm colors it.
    #[inline(never)]
    fn regexp_arg(&self, p: usize) -> bool {
        let b = self.b;
        let mut k = p;
        while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
            k -= 1;
        }
        if k == 0 || b[k - 1] != b'(' {
            return false;
        }
        k -= 1;
        while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
            k -= 1;
        }
        k >= 6
            && &b[k - 6..k] == b"RegExp"
            && (k == 6 || !(b[k - 7].is_ascii_alphanumeric() || matches!(b[k - 7], b'_' | b'$')))
    }

    /// A regex string's turn ([`RE_STR`], in context `top`): the host string's or template's
    /// patterns first (its end, an escape but `\\`, which is the regex's `\`, a `${`), then the
    /// regex.
    fn regex_string(&mut self, p: usize, top: u8) -> Option<usize> {
        let host = self.stack[self.depth - 2];
        let c = self.c(p);
        let which = if host == TPL {
            match c {
                Some(b'`') => {
                    self.pop();
                    return self.close(p, p + 1, TPL_END);
                }
                Some(b'\\') if self.c(p + 1) != Some(b'\\') => {
                    if let Some(q) = self.string_content(p, ESC, ESC_NL) {
                        return Some(q);
                    }
                    if self.is(p + 1, b'`') {
                        return self.tok(p, p + 2, ESC);
                    }
                }
                Some(b'$') if self.is(p + 1, b'{') => {
                    return self.tok_push(p, p + 2, SUBST_BEGIN, SUBST);
                }
                _ => {}
            }
            2
        } else {
            let q = if host == STR_D { b'"' } else { b'\'' };
            if c == Some(q) {
                self.pop();
                return self.close(p, p + 1, STR_END);
            }
            if p == self.b.len() {
                // `(\n)`: the string ends with its line
                self.fill(p);
                self.pop();
                self.pop();
                return Some(p + 1);
            }
            if c == Some(b'\\')
                && self.c(p + 1) != Some(b'\\')
                && let Some(e) = self.string_content(p, STR_ESC, STR_ESC_NL)
            {
                return Some(e);
            }
            usize::from(host == STR_S)
        };
        self.k.close(self.out);
        let r = regex::embedded(
            self.b,
            p,
            top & regex::ECMA_STATE,
            &RE_STR_EMBED[which],
            &mut self.at,
            self.out,
        );
        self.k.reopen(self.out, 16);
        let (q, st) = r?;
        self.set_top(RE_STR | st);
        Some(q)
    }

    /// A template's text.
    fn template(&mut self, p: usize) -> Option<usize> {
        let c = self.c(p)?;
        if c == b'`' {
            return self.close(p, p + 1, TPL_END);
        }
        if c == b'\\' {
            if let Some(q) = self.string_content(p, ESC, ESC_NL) {
                return Some(q);
            }
            if self.is(p + 1, b'`') {
                return self.tok(p, p + 2, ESC);
            }
        }
        if has(self.b, p, b"${") {
            return self.tok_push(p, p + 2, SUBST_BEGIN, SUBST);
        }
        if p >= self.b.len() {
            return None;
        }
        // `.`: a character; a run of them is one span
        let mut e = self.next(p);
        while e < self.b.len() && !matches!(self.b[e], b'`' | b'\\' | b'$') {
            e += 1;
        }
        self.tok(p, e, TPL_TEXT)
    }

    /// `string-content` at `p`: `\\\s*\n`, then an escape.
    fn string_content(&mut self, p: usize, esc: u16, nl: u16) -> Option<usize> {
        if !self.is(p, b'\\') {
            return None;
        }
        let s = spaces(self.b, p + 1);
        if s > self.b.len() {
            // the spaces took the line's `\n`
            return self.tok(p, s, nl);
        }
        let e = escape(self.b, p)?;
        self.tok(p, e, esc)
    }

    /// `literal-string`.
    fn string(&mut self, p: usize) -> Option<usize> {
        let (q, ctx) = match self.c(p)? {
            b'"' => (b'"', STR_D),
            b'\'' => (b'\'', STR_S),
            _ => return None,
        };
        let e = self.tok_push(p, p + 1, STR_BEGIN, ctx)?;
        if after_paren(self.b, p) && self.regexp_arg(p) && self.can_push() {
            self.push(RE_STR);
            return Some(e);
        }
        // a string that closes on its line before any `\\`: its text and its end in this step,
        // as the string's own dispatches would give them
        let b = self.b;
        let i = crate::prim::find_in(b, e, b.len(), [q, b'\\']);
        if i < self.b.len() && b[i] == q {
            return self.close(i, i + 1, STR_END);
        }
        Some(e)
    }

    /// `literal-language-constant`.
    fn language_constant(&mut self, p: usize) -> Option<usize> {
        if !self.not_after_dot(p) {
            return None;
        }
        let w = self.word(p)?;
        let leaf = match w {
            b"true" => TRUE,
            b"false" => FALSE,
            b"null" => NULL,
            b"undefined" => UNDEFINED,
            b"NaN" => NAN,
            _ => return None,
        };
        self.tok(p, p + w.len(), leaf)
    }

    /// `literal-language-variable`.
    fn language_variable(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let after_dot = before(b, p) == Some(b'.');
        if after_dot {
            // `(?<=\.)(__proto__)\b` and the like
            for (w, leaf) in [
                (&b"__proto__"[..], PROTO),
                (b"constructor", CONSTRUCTOR),
                (b"prototype", PROTOTYPE),
            ] {
                if has(b, p, w) && boundary(b, p + w.len()) {
                    return self.tok(p, p + w.len(), leaf);
                }
            }
        }
        // `(?<!(?<!\.\.)\.)`: after a `.` only when it ends a `...`
        if after_dot && !(p >= 3 && &b[p - 3..p - 1] == b"..") {
            return None;
        }
        let w = self.word(p)?;
        let leaf = match w {
            b"arguments" => ARGUMENTS,
            b"super" => SUPER,
            b"this" => THIS,
            b"self" => SELF,
            _ => return None,
        };
        self.tok(p, p + w.len(), leaf)
    }

    /// `literal-constructor`: `(new)\s+(?=[_$a-zA-Z][$\w.]*)`.
    fn constructor(&mut self, p: usize) -> Option<usize> {
        if !has(self.b, p, b"new") {
            return None;
        }
        space_len(self.b, p + 3)?;
        let s = spaces(self.b, p + 3);
        if !ident_start(self.c(s)) {
            return None;
        }
        if !self.can_push() {
            return None;
        }
        self.put(p, p + 3, NEW);
        self.put(s, s, PLAIN);
        self.push(CTOR);
        Some(s)
    }

    /// `literal-method-call`.
    fn method_call(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let after_dot = before(b, p) == Some(b'.');
        // `(?:(?<=\.)|\b)([A-Z][$\w]*)\s*(\.)([_$a-zA-Z][$\w]*)\s*…`
        if (after_dot || boundary(b, p)) && self.c(p).is_some_and(|c| c.is_ascii_uppercase()) {
            let e = name_end(b, p + 1);
            let d = spaces(b, e);
            if self.is(d, b'.')
                && let Some(m) = ident(b, d + 1)
            {
                let s = spaces(b, m);
                if self.is(s, b'(') {
                    self.put(p, e, CLASS_VAR);
                    self.put(d, d + 1, ACCESSOR);
                    self.put(d + 1, m, FUNCTION);
                    let c = spaces(b, s + 1);
                    if self.is(c, b')') {
                        return self.tok(s, c + 1, CALL_PARENS);
                    }
                    return Some(s);
                }
            }
        }
        if !after_dot {
            return None;
        }
        let e = ident(b, p)?;
        let s = spaces(b, e);
        if !self.is(s, b'(') {
            return None;
        }
        self.put(p, e, FUNCTION);
        let c = spaces(b, s + 1);
        if self.is(c, b')') {
            return self.tok(s, c + 1, CALL_PARENS);
        }
        Some(s)
    }

    /// `literal-function-call`.
    fn function_call(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let e = ident(b, p)?;
        let s = spaces(b, e);
        match self.c(s) {
            Some(b'(') => {
                self.put(p, e, FUNCTION);
                let c = spaces(b, s + 1);
                if self.is(c, b')') {
                    return self.tok(s, c + 1, CALL_PARENS);
                }
                Some(s)
            }
            Some(b'`') => {
                self.put(p, e, FUNCTION);
                Some(s)
            }
            _ => None,
        }
    }

    /// `comments`.
    fn comments(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        match self.c(p)? {
            b'/' => match self.c(p + 1) {
                Some(b'*') => {
                    if self.is(p + 2, b'*') && !self.is(p + 3, b'/') {
                        return self.tok_push(p, p + 3, DOC_MARK, DOC);
                    }
                    self.tok_push(p, p + 2, BLOCK_BEGIN, BLOCK)
                }
                Some(b'/') => {
                    self.put(p, p + 2, LINE_MARK);
                    self.tok(p + 2, self.b.len(), LINE)?;
                    Some(self.b.len() + 1)
                }
                _ => None,
            },
            b'<' if has(b, p, b"<!--") => self.tok(p, p + 4, HTML_COMMENT),
            b'-' if has(b, p, b"-->") => self.tok(p, p + 3, HTML_COMMENT),
            b'#' if p == 0 && self.is(1, b'!') => {
                self.put(0, 2, SHEBANG_MARK);
                self.tok(2, self.b.len(), SHEBANG)?;
                Some(self.b.len() + 1)
            }
            _ => None,
        }
    }

    /// `brackets`.
    fn brackets(&mut self, p: usize) -> Option<usize> {
        match self.c(p)? {
            b'(' => self.tok_push(p, p + 1, ROUND_BEGIN, ROUND),
            b'[' => self.tok_push(p, p + 1, SQUARE_BEGIN, SQUARE),
            b'{' => self.tok_push(p, p + 1, CURLY_BEGIN, CURLY),
            _ => None,
        }
    }

    /// `literal-punctuation`.
    fn punctuation(&mut self, p: usize) -> Option<usize> {
        match self.c(p)? {
            b';' => self.tok(p, p + 1, TERMINATOR),
            b',' => self.tok(p, p + 1, COMMA),
            _ => None,
        }
    }

    /// `literal-operators`.
    fn operators(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        if self.c(p).is_some_and(|c| WORD[usize::from(c)] || c >= 0x80)
            && let Some(e) = self.keyword(
                p,
                &[
                    b"delete",
                    b"instanceof",
                    b"in",
                    b"new",
                    b"of",
                    b"typeof",
                    b"void",
                    b"with",
                ],
            )
        {
            return self.tok(p, e, OP);
        }
        let c = self.c(p)?;
        let at = |i: usize| self.c(p + i);
        let (len, leaf) = match c {
            b'!' if at(1) != Some(b'=') => (1, LOGICAL),
            b'&' if at(1) == Some(b'&') => (2, LOGICAL),
            b'|' if at(1) == Some(b'|') => (2, LOGICAL),
            b'=' if at(1) != Some(b'=') => (1, ASSIGN),
            _ => {
                // augmented assignments
                const AUG: [&[u8]; 11] = [
                    b"%=", b"&=", b"*=", b"+=", b"-=", b"/=", b"^=", b"|=", b"<<=", b">>=", b">>>=",
                ];
                if let Some(a) = AUG.iter().find(|a| has(b, p, a)) {
                    (a.len(), AUGMENTED)
                } else if c == b'~' {
                    (1, BITWISE)
                } else if has(b, p, b"<<") {
                    (2, BITWISE)
                } else if has(b, p, b">>>") {
                    (3, BITWISE)
                } else if has(b, p, b">>") {
                    (2, BITWISE)
                } else if matches!(c, b'&' | b'^' | b'|') {
                    (1, BITWISE)
                } else if has(b, p, b"<=") || has(b, p, b">=") {
                    (2, RELATIONAL)
                } else if matches!(c, b'<' | b'>') {
                    (1, RELATIONAL)
                } else if has(b, p, b"===") || has(b, p, b"!==") {
                    (3, COMPARISON)
                } else if has(b, p, b"==") || has(b, p, b"!=") {
                    (2, COMPARISON)
                } else if has(b, p, b"--") || has(b, p, b"++") {
                    (2, ARITHMETIC)
                } else if matches!(c, b'/' | b'%' | b'*' | b'+' | b'-') {
                    (1, ARITHMETIC)
                } else if matches!(c, b'?' | b':') {
                    (1, TERNARY)
                } else if has(b, p, b"...") && self.not_after_dot(p) {
                    (3, SPREAD)
                } else if c == b'.' {
                    (1, ACCESSOR)
                } else {
                    return None;
                }
            }
        };
        self.tok(p, p + len, leaf)
    }

    /// `literal-variable`.
    fn variable(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        let c = self.c(p)?;
        // `_*?[A-Z][_$\dA-Z]*\b`
        if c == b'_' || c.is_ascii_uppercase() {
            let mut u = p;
            while self.is(u, b'_') {
                u += 1;
            }
            if self.c(u).is_some_and(|c| c.is_ascii_uppercase()) {
                let mut e = u + 1;
                loop {
                    match self.c(e) {
                        Some(c) if c == b'_' || c == b'$' || c.is_ascii_uppercase() => e += 1,
                        Some(c) if c.is_ascii_digit() => e += 1,
                        Some(c) if c >= 0x80 && digit(b, e).is_some() => {
                            e = digit(b, e).unwrap_or(e + 1);
                        }
                        _ => break,
                    }
                }
                // backs off to a `\b`
                while e > u && !boundary(b, e) {
                    e -= 1;
                    while e > u && b[e] & 0xc0 == 0x80 {
                        e -= 1;
                    }
                }
                if e > u && boundary(b, e) {
                    return self.tok(p, e, CONSTANT);
                }
            }
        }
        // `\b([A-Z][$\w]*)\s*(\.)([_$a-zA-Z][$\w]*)`
        if c.is_ascii_uppercase() && boundary(b, p) {
            let e = name_end(b, p + 1);
            let d = spaces(b, e);
            if self.is(d, b'.')
                && let Some(m) = ident(b, d + 1)
            {
                self.put(p, e, CLASS_VAR);
                self.put(d, d + 1, ACCESSOR);
                return self.tok(d + 1, m, STATIC_PROP);
            }
        }
        let after_dot = before(b, p) == Some(b'.');
        // `(?<!\.)([_$a-zA-Z][$\w]*)\s*(?=[\[\.])`
        if !after_dot && let Some(e) = ident(b, p) {
            let s = spaces(b, e);
            if matches!(self.c(s), Some(b'[' | b'.')) {
                return self.tok(p, s, OBJECT);
            }
        }
        // `(?<=\.)\s*([_$a-zA-Z][$\w]*)`
        if after_dot {
            let s = self.spaces_at(p);
            if let Some(e) = ident(b, s) {
                self.put(p, s, PLAIN);
                return self.tok(s, e, PROPERTY);
            }
        }
        let e = ident(b, p)?;
        self.tok(p, e, READWRITE)
    }

    // --- classes ---

    /// `class-properties`: `([_$a-zA-Z][$\w]*)(?=\s*[:=])`.
    fn class_property(&mut self, p: usize) -> Option<usize> {
        let e = ident(self.b, p)?;
        let s = spaces(self.b, e);
        if !matches!(self.c(s), Some(b':' | b'=')) {
            return None;
        }
        self.tok_push(p, e, PROPERTY, CLASS_PROP)
    }

    /// `class-method-definition`.
    fn method_definition(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // `(@@[_$a-zA-Z][$\w]*|static|return)(?=\s*[<(])|(?=\s*<)`
        let head = if has(b, p, b"@@") {
            ident(b, p + 2)
        } else if has(b, p, b"static") || has(b, p, b"return") {
            Some(p + 6)
        } else {
            None
        };
        if let Some(e) = head
            && matches!(self.c(spaces(b, e)), Some(b'<' | b'('))
        {
            return self.tok_push(p, e, FLOW_OP, METHOD_DEF);
        }
        if self.is(self.spaces_at(p), b'<') {
            return self.enter(p, METHOD_DEF);
        }
        // `(?<=[]"'])\s*(?=[<(])`
        if matches!(before(b, p), Some(b']' | b'"' | b'\'')) {
            let s = self.spaces_at(p);
            if matches!(self.c(s), Some(b'<' | b'(')) {
                if s == p {
                    return self.enter(p, METHOD_DEF);
                }
                return self.tok_push(p, s, PLAIN, METHOD_DEF);
            }
        }
        // `((?>get|set)\s+)(?>KEY)(?=\s*[<(])`
        if (has(b, p, b"get") || has(b, p, b"set")) && space_len(b, p + 3).is_some() {
            let k = spaces(b, p + 3);
            if let Some(key) = key(b, k)
                && matches!(self.c(spaces(b, key.end)), Some(b'<' | b'('))
            {
                if !self.can_push() {
                    return None;
                }
                self.put(p, k, STORAGE);
                self.key_caps(k, key);
                self.push(METHOD_DEF);
                return Some(key.end);
            }
        }
        // `(?>KEY)(?=\s*[<(])`
        let key = key(b, p)?;
        if !matches!(self.c(spaces(b, key.end)), Some(b'<' | b'(')) || !self.can_push() {
            return None;
        }
        self.key_caps(p, key);
        self.push(METHOD_DEF);
        Some(key.end)
    }

    /// `class-method-storage`.
    fn method_storage(&mut self, p: usize) -> Option<usize> {
        if self.is(p, b'*') {
            return self.tok(p, p + 1, ASTERISK);
        }
        if !self.not_after_dot(p) {
            return None;
        }
        let w = self.word(p)?;
        let e = p + w.len();
        match w {
            b"static" | b"declare" => self.tok(p, e, MODIFIER),
            b"async" => self.tok(p, e, STORAGE),
            b"get" | b"set" if !self.is(spaces(self.b, e), b'(') => self.tok(p, e, STORAGE),
            _ => None,
        }
    }

    // --- functions ---

    /// `function-declaration-parameters`' `(`.
    fn parameters(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'(') {
            return None;
        }
        self.tok_push(p, p + 1, PARAMS_BEGIN, PARAMS)
    }

    /// A parameter list's patterns.
    fn params(&mut self, p: usize) -> Option<usize> {
        let c = self.c(p)?;
        if c == b')' {
            return self.close(p, p + 1, PARAMS_END);
        }
        if let Some(q) = self.comments(p).or_else(|| self.flow_annotation(p)) {
            return Some(q);
        }
        if has(self.b, p, b"...") && self.not_after_dot(p) {
            return self.tok(p, p + 3, SPREAD);
        }
        if let Some(e) = ident(self.b, p) {
            return self.tok(p, e, PARAM);
        }
        match c {
            b',' => self.tok(p, p + 1, PARAM_SEP),
            b'=' => self.tok_push(p, p + 1, ASSIGN, PARAM_DEFAULT),
            _ => None,
        }
    }

    // --- Flow ---

    /// `flowtype-annotation`: `(?:(\?)\s*)?(:)`.
    fn flow_annotation(&mut self, p: usize) -> Option<usize> {
        let (q, c) = if self.is(p, b'?') {
            let s = spaces(self.b, p + 1);
            if self.is(s, b':') {
                (Some(p), s)
            } else {
                (None, p)
            }
        } else {
            (None, p)
        };
        if !self.is(c, b':') || !self.can_push() {
            return None;
        }
        if let Some(q) = q {
            self.put(q, q + 1, FLOW_OPTIONAL);
        }
        self.tok_push(c, c + 1, FLOW_ANNOTATION, FLOW_ANNOT)
    }

    /// `flowtype-polymorph`'s `<`.
    fn polymorph(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'<') {
            return None;
        }
        self.tok_push(p, p + 1, FLOW_BEGIN, FLOW_ANGLE)
    }

    /// `flowtype-brackets`' `{`.
    fn flow_brackets(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'{') {
            return None;
        }
        self.tok_push(p, p + 1, FLOW_BEGIN, FLOW_BRACES)
    }

    /// `flowtype-tokens`.
    fn flow_tokens(&mut self, p: usize) -> Option<usize> {
        let b = self.b;
        // `(?<=[:?|&=])(?=\s*{)`
        if matches!(before(b, p), Some(b':' | b'?' | b'|' | b'&' | b'='))
            && self.is(self.spaces_at(p), b'{')
        {
            return self.enter(p, FLOW_AFTER);
        }
        // `\s*([|&])\s*`
        let s = self.spaces_at(p);
        if matches!(self.c(s), Some(b'|' | b'&')) {
            let e = spaces(b, s + 1);
            self.put(p, s, PLAIN);
            self.put(s, s + 1, FLOW_OTHER);
            return Some(e.min(self.b.len() + 1));
        }
        let c = self.c(p)?;
        match c {
            b'*' | b':' | b'?' | b'&' | b'|' | b'.' => return self.tok(p, p + 1, FLOW_OTHER),
            b'<' => return self.tok_push(p, p + 1, FLOW_BEGIN, FLOW_ANGLE),
            b'[' => return self.tok_push(p, p + 1, FLOW_BEGIN, FLOW_SQUARE),
            b'(' => return self.tok_push(p, p + 1, FLOW_BEGIN, FLOW_ROUND),
            _ => {}
        }
        if self.word(p) == Some(b"typeof") {
            return self.tok(p, p + 6, FLOW_OTHER);
        }
        if has(b, p, b"=>") {
            return self.tok_push(p, p + 2, FLOW_OP, FLOW_FN);
        }
        self.flow_identifier(p)
            .or_else(|| self.string(p))
            .or_else(|| self.comments(p))
    }

    /// `flowtype-identifier`.
    fn flow_identifier(&mut self, p: usize) -> Option<usize> {
        if let Some(q) = self.support_class(p) {
            return Some(q);
        }
        if let Some(w) = self.word(p)
            && matches!(
                w,
                b"any" | b"bool" | b"boolean" | b"mixed" | b"number" | b"string" | b"void"
            )
        {
            return self.tok(p, p + w.len(), FLOW_PRIMITIVE);
        }
        if let Some(e) = ident(self.b, p) {
            return self.tok(p, e, FLOW_VAR);
        }
        if self.is(p, b'?') {
            return self.tok(p, p + 1, FLOW_OPTIONAL);
        }
        None
    }

    // --- JSX ---

    /// `jsx-tag-start`: `(<)([_$a-zA-Z][-$:.\w]*[$\w]*)`, or a lone `<`.
    fn jsx_tag_start(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'<') {
            return None;
        }
        if ident_start(self.c(p + 1)) {
            let b = self.b;
            let mut e = p + 2;
            loop {
                match self.c(e) {
                    Some(c) if c < 0x80 => {
                        if WORD[usize::from(c)] || matches!(c, b'-' | b'$' | b':' | b'.') {
                            e += 1;
                        } else {
                            break;
                        }
                    }
                    Some(_) if e < self.b.len() && is_word(b, e) => e += char_len(b[e]),
                    _ => break,
                }
            }
            if !self.can_push() {
                return None;
            }
            self.put(p, p + 1, TAG_BEGIN);
            self.put(p + 1, e, TAG_NAME);
            self.push(JSX_TAG | tag_hash(&b[p + 1..e]));
            self.tags[self.tag_count % TAGS] = ((self.depth - 1) as u8, p as u32 + 1, e as u32);
            self.tag_count += 1;
            return Some(e);
        }
        self.tok(p, p + 1, TAG_INCOMPLETE)
    }

    /// A JSX element's opening tag and its attributes, until it closes.
    fn jsx_tag(&mut self, p: usize, top: u8) -> Option<usize> {
        let b = self.b;
        let c = self.c(p)?;
        // `(</)(\2)(>)|(/>)`
        if has(b, p, b"</") {
            let level = self.depth - 1;
            // the tag pushed at this level on this line, the latest first
            let recent = (0..self.tag_count.min(TAGS))
                .map(|i| self.tags[(self.tag_count - 1 - i) % TAGS])
                .find(|&(l, _, _)| usize::from(l) == level);
            let name_end = if level >= self.low
                && let Some((_, s, e)) = recent
            {
                let name = &b[s as usize..e as usize];
                has(b, p + 2, name).then(|| p + 2 + name.len())
            } else {
                // from an earlier line: the text up to `>` with the name's hash
                let e = b[p + 2..]
                    .iter()
                    .position(|&c| c == b'>')
                    .map(|i| p + 2 + i);
                e.filter(|&e| e > p + 2 && tag_hash(&b[p + 2..e]) == top & !JSX_TAG)
            };
            if let Some(e) = name_end
                && self.is(e, b'>')
            {
                self.put(p, p + 2, TAG_BEGIN);
                self.put(p + 2, e, TAG_NAME);
                return self.close(e, e + 1, TAG_END);
            }
        }
        if has(b, p, b"/>") {
            return self.close(p, p + 2, TAG_END);
        }
        match c {
            // `jsx-tag-end`
            b'>' => self.tok_push(p, p + 1, TAG_END, JSX_CHILDREN),
            b'=' => self.tok(p, p + 1, ATTR_EQ),
            b'"' => self.tok_push(p, p + 1, JSTR_BEGIN, JSTR_D),
            b'\'' => self.tok_push(p, p + 1, JSTR_BEGIN, JSTR_S),
            b'{' => self.jsx_evaluated(p),
            _ => {
                // `jsx-attribute-name`: `[_$a-zA-Z][-$\w]*`
                if ident_start(Some(c)) {
                    let mut e = p + 1;
                    loop {
                        e = name_end(b, e);
                        if self.is(e, b'-') {
                            e += 1;
                        } else {
                            break;
                        }
                    }
                    return self.tok(p, e, ATTR);
                }
                self.comments(p)
            }
        }
    }

    /// `jsx-evaluated-code`'s `{`.
    fn jsx_evaluated(&mut self, p: usize) -> Option<usize> {
        if !self.is(p, b'{') {
            return None;
        }
        self.tok_push(p, p + 1, EMBED_BEGIN, JSX_EXPR)
    }

    /// `jsx-entities`: `&(?:[a-zA-Z0-9]+|#\d+|#x\h+);`, then `&\S*;`.
    fn jsx_entities(&mut self, p: usize, entity: u16, bad: u16) -> Option<usize> {
        if !self.is(p, b'&') {
            return None;
        }
        let b = self.b;
        let run = |from: usize, ok: fn(&u8) -> bool| {
            from + b
                .get(from..)
                .map_or(0, |r| r.iter().take_while(|c| ok(c)).count())
        };
        let alnum = run(p + 1, u8::is_ascii_alphanumeric);
        let e = if alnum > p + 1 {
            Some(alnum)
        } else if self.is(p + 1, b'#') {
            if self.is(p + 2, b'x') && run(p + 3, u8::is_ascii_hexdigit) > p + 3 {
                // `#\d+` is tried first: `&#x…` has no digit after `#`
                Some(run(p + 3, u8::is_ascii_hexdigit))
            } else {
                digits(b, p + 2)
            }
        } else {
            None
        };
        if let Some(e) = e
            && self.is(e, b';')
        {
            return self.tok(p, e + 1, entity);
        }
        // `&\S*;`: the last `;` before a space
        let mut s = p + 1;
        while s < self.b.len() && space_len(b, s).is_none() {
            s += char_len(b[s]);
        }
        let semi = b[p + 1..s.min(self.b.len())]
            .iter()
            .rposition(|&c| c == b';')?;
        self.tok(p, p + 1 + semi + 1, bad)
    }

    // --- regex literals ---

    /// A regex literal's body from `p`, where its end didn't match, in context `top`: the
    /// regex to where its end can (a `/` outside its groups and classes) or past the line.
    fn regex(&mut self, p: usize, top: u8) -> Option<usize> {
        self.k.close(self.out);
        let r = regex::embedded(
            self.b,
            p,
            top & regex::ECMA_STATE,
            &RE_EMBED,
            &mut self.at,
            self.out,
        );
        self.k.reopen(self.out, 16);
        let (q, st) = r?;
        self.set_top(REGEX | st);
        Some(q)
    }
}

/// Whether a `(` is right before `p`, or one blank and a `(` (`RegExp( "…")`): where a string may
/// be `RegExp(`'s argument, tested inline before [`Scan::regexp_arg`] reads further back.
#[inline]
fn after_paren(b: &[u8], p: usize) -> bool {
    match p {
        0 => false,
        1 => b[0] == b'(',
        _ => b[p - 1] == b'(' || (b[p - 1] == b' ' && b[p - 2] == b'('),
    }
}

#[cfg(test)]
mod tests {
    use crate::Lang;
    use crate::tests::marked;

    fn js(lines: &[&'static str]) -> Vec<(&'static str, &'static str)> {
        marked(Lang::JavaScript, lines)
    }

    /// The parentheses matched in one pass end where [`super::balanced`]'s read from each `(`
    /// does (X-078), on every line of `(`, `)` and `a` up to nine bytes.
    #[test]
    fn paren_ends_are_balanced_at_every_paren() {
        let mut line = Vec::new();
        let mut ends = Vec::new();
        for n in 0..=9u32 {
            for k in 0..3usize.pow(n) {
                line.clear();
                let mut k = k;
                for _ in 0..n {
                    line.push(b"()a"[k % 3]);
                    k /= 3;
                }
                super::paren_ends(&line, &mut ends);
                for p in (0..line.len()).filter(|&p| line[p] == b'(') {
                    let e = (ends[p] != super::NO_END).then_some(ends[p] as usize);
                    assert_eq!(e, super::balanced(&line, p), "{line:?} at {p}");
                }
            }
        }
    }

    /// [`super::Scan::balanced`] on long lines, where its reads pass the budget and the line's
    /// table answers from then on (X-078), is the read from each place, asked forward, backward
    /// and again.
    #[test]
    fn balanced_past_its_budget_is_the_read_from_each_place() {
        // the line, and whether its reads pass the budget
        let mut lines: Vec<(Vec<u8>, bool)> = [
            ("(", true),
            ("(()", true),
            ("f(", true),
            ("(a)(", true),
            ("((x))", false),
            (")(", false),
        ]
        .iter()
        .map(|&(p, long)| (p.repeat(400).into_bytes(), long))
        .collect();
        // and a mix, from a small generator
        let mut x = 0x2545_f491_4f6c_dd1du64;
        lines.push((
            (0..3000)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    b"(()a"[(x % 4) as usize]
                })
                .collect(),
            true,
        ));
        for (line, long) in &lines {
            let (state, mut spans) = (crate::State::default(), Vec::new());
            let mut out = crate::Out::new(&mut spans);
            let s = super::Scan::fresh(line, 0, &state, &mut out, crate::emit::Push);
            let all = 0..=line.len() + 1;
            for p in all.clone().chain(all.clone().rev()).chain(all) {
                assert_eq!(s.balanced(p), super::balanced(line, p), "at {p}");
            }
            let table = s.paren_read.get() == usize::MAX;
            assert_eq!(table, *long, "{:?}…", &line[..8]);
        }
    }

    /// The run of blanks kept from a place is the read from each place in it (X-078), asked
    /// forward, backward and again, on lines of ASCII blanks, spaces past ASCII (U+2001, U+3000)
    /// and others, a character's inner bytes included.
    #[test]
    fn spaces_at_is_the_read_from_each_place() {
        let pieces = [" ", "\t", "\u{2001}", "\u{3000}", "a", "\u{e9}", ":"];
        for n in 0..=5u32 {
            for k in 0..pieces.len().pow(n) {
                let mut line = String::new();
                let mut k = k;
                for _ in 0..n {
                    line.push_str(pieces[k % pieces.len()]);
                    k /= pieces.len();
                }
                let b = line.as_bytes();
                let (state, mut spans) = (crate::State::default(), Vec::new());
                let mut out = crate::Out::new(&mut spans);
                let s = super::Scan::fresh(b, 0, &state, &mut out, crate::emit::Push);
                let all = 0..=b.len() + 1;
                for p in all.clone().chain(all.clone().rev()).chain(all) {
                    assert_eq!(s.spaces_at(p), super::spaces(b, p), "{line:?} at {p}");
                }
            }
        }
    }

    /// Every word a pattern at a name's start looks for has that pattern's bit: a word without
    /// it skips the pattern.
    /// The word table's masks (X-058) are those of the `match` it replaced.
    #[test]
    fn the_word_tables_masks_are_the_matchs() {
        use super::*;
        #[allow(clippy::match_same_arms)]
        fn old_mask(w: &[u8]) -> u16 {
            // one `match`: the compiler tells the words apart by length and bytes
            match w {
                b"Array" | b"ArrayBuffer" | b"Boolean" | b"DataView" | b"Date"
                | b"Float32Array" | b"Float64Array" | b"Function" | b"Int16Array"
                | b"Int32Array" | b"Int8Array" | b"JSON" | b"Map" | b"Math" | b"Number"
                | b"Object" | b"Promise" | b"Proxy" | b"Reflect" | b"RegExp" | b"Set"
                | b"String" | b"Symbol" | b"System" | b"TypeError" | b"Uint16Array"
                | b"Uint32Array" | b"Uint8Array" | b"Uint8ClampedArray" | b"WeakMap"
                | b"WeakSet" | b"Error" | b"EvalError" | b"RangeError" | b"ReferenceError"
                | b"SyntaxError" | b"URIError" | b"Buffer" => W_SUPPORT_CLASS,
                b"Infinity" => W_SUPPORT_CLASS | W_NUMBER,
                b"NaN" => W_SUPPORT_CLASS | W_CONSTANT,
                b"constructor" => W_SUPPORT_OTHER | W_LANG_VAR,
                b"decodeURI"
                | b"decodeURIComponent"
                | b"encodeURI"
                | b"encodeURIComponent"
                | b"escape"
                | b"eval"
                | b"hasOwnProperty"
                | b"isFinite"
                | b"isNaN"
                | b"isPrototypeOf"
                | b"parseFloat"
                | b"parseInt"
                | b"propertyIsEnumerable"
                | b"toLocaleString"
                | b"toString"
                | b"unescape"
                | b"valueOf"
                | b"clearImmediate"
                | b"clearInterval"
                | b"clearTimeout"
                | b"require"
                | b"setImmediate"
                | b"setInterval"
                | b"setTimeout"
                | b"document"
                | b"window"
                | b"global"
                | b"GLOBAL"
                | b"root"
                | b"__dirname"
                | b"__filename"
                | b"console"
                | b"process"
                | b"exports" => W_SUPPORT_OTHER,
                b"module" => W_SUPPORT_OTHER | W_KEYWORD,
                b"const" | b"let" | b"var" | b"await" | b"yield" | b"return" | b"if" | b"else"
                | b"catch" | b"finally" | b"throw" | b"try" | b"break" | b"continue" | b"do"
                | b"goto" | b"while" | b"case" | b"enum" | b"public" | b"package" | b"private"
                | b"protected" | b"debugger" => W_KEYWORD,
                b"default" => W_KEYWORD | W_MODULE,
                b"interface" => W_KEYWORD | W_CLASS,
                b"import" | b"export" | b"from" | b"as" => W_MODULE,
                b"class" => W_CLASS,
                b"declare" | b"type" => W_FLOW,
                b"true" | b"false" | b"null" | b"undefined" => W_CONSTANT,
                b"arguments" | b"super" | b"this" | b"self" | b"__proto__" | b"prototype" => {
                    W_LANG_VAR
                }
                b"new" => W_NEW | W_OPERATOR,
                b"delete" | b"instanceof" | b"in" | b"of" | b"typeof" | b"void" | b"with" => {
                    W_OPERATOR
                }
                b"async" | b"function" | b"static" | b"get" | b"set" => W_OTHER,
                b"for" | b"switch" => W_FOR_SWITCH,
                b"styled" | b"injectGlobal" | b"keyframes" => W_STYLED,
                b"gql" => W_GQL,
                _ => 0,
            }
        }

        for w in super::code::LIST_WORDS {
            assert_eq!(
                super::word_mask(w),
                old_mask(w),
                "{}",
                String::from_utf8_lossy(w)
            );
        }
        for w in [&b"foo"[..], b"Foo", b"thi", b"thiss", b"Arra", b"", b"x"] {
            assert_eq!(super::word_mask(w), old_mask(w));
        }
    }

    #[test]
    fn the_word_mask_names_every_pattern_a_word_can_start() {
        use super::*;
        let sets: &[(&[&[u8]], u16)] = &[
            (BUILTIN_CLASSES, W_SUPPORT_CLASS),
            (ERRORS, W_SUPPORT_CLASS),
            (&[b"Buffer"], W_SUPPORT_CLASS),
            (BUILTIN_FNS, W_SUPPORT_OTHER),
            (NODE_FNS, W_SUPPORT_OTHER),
            (
                &[
                    b"document",
                    b"window",
                    b"global",
                    b"GLOBAL",
                    b"root",
                    b"__dirname",
                    b"__filename",
                    b"console",
                    b"process",
                    b"exports",
                    b"module",
                ],
                W_SUPPORT_OTHER,
            ),
            (
                &[
                    b"const",
                    b"let",
                    b"var",
                    b"await",
                    b"yield",
                    b"return",
                    b"if",
                    b"else",
                    b"catch",
                    b"finally",
                    b"throw",
                    b"try",
                    b"break",
                    b"continue",
                    b"do",
                    b"goto",
                    b"while",
                    b"case",
                    b"default",
                    b"enum",
                    b"module",
                    b"public",
                    b"package",
                    b"private",
                    b"interface",
                    b"protected",
                    b"debugger",
                ],
                W_KEYWORD,
            ),
            (
                &[b"import", b"export", b"default", b"from", b"as"],
                W_MODULE,
            ),
            (&[b"class", b"interface"], W_CLASS),
            (&[b"declare", b"type"], W_FLOW),
            (&[b"Infinity"], W_NUMBER),
            (
                &[b"true", b"false", b"null", b"undefined", b"NaN"],
                W_CONSTANT,
            ),
            (
                &[
                    b"arguments",
                    b"super",
                    b"this",
                    b"self",
                    b"__proto__",
                    b"constructor",
                    b"prototype",
                ],
                W_LANG_VAR,
            ),
            (&[b"new"], W_NEW),
            (
                &[
                    b"delete",
                    b"instanceof",
                    b"in",
                    b"new",
                    b"of",
                    b"typeof",
                    b"void",
                    b"with",
                ],
                W_OPERATOR,
            ),
            (&[b"for", b"switch"], W_FOR_SWITCH),
            (&[b"styled", b"injectGlobal", b"keyframes"], W_STYLED),
            (&[b"gql"], W_GQL),
        ];
        for &(words, bit) in sets {
            for w in words {
                assert_ne!(
                    word_mask(w) & bit,
                    0,
                    "{} lacks {bit:#x}",
                    String::from_utf8_lossy(w)
                );
            }
        }
        for w in [&b"async"[..], b"function", b"static", b"get", b"set"] {
            assert_ne!(word_mask(w) & W_OTHER, 0);
        }
        assert_eq!(word_mask(b"foo"), 0);
        assert_eq!(word_mask(b"Foo"), 0);
    }

    #[test]
    fn a_function_and_its_parts() {
        let m = js(&["function foo(a, b = 1) { return a / b; }"]);
        assert!(
            m.contains(&("function", "storage.type.function.js")),
            "{m:?}"
        );
        assert!(m.contains(&("foo", "entity.name.function.js")), "{m:?}");
        assert!(
            m.contains(&("a", "variable.parameter.function.js")),
            "{m:?}"
        );
        assert!(m.contains(&("return", "keyword.control.flow.js")), "{m:?}");
        assert!(
            m.contains(&("/", "keyword.operator.arithmetic.js")),
            "{m:?}"
        );
    }

    #[test]
    fn a_regex_literals_body_is_ecmascripts_regex() {
        let has = |m: &[(&str, &str)], text: &str, scopes: &str| {
            assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
        };
        // a `/` in a class, escaped or in a group doesn't end it
        let m = js(&[r"x = /^a[/.]b\/(?<n>c/d)\k<n>{2,}?\u{1F600}\p{L}\A/gsu;"]);
        has(
            &m,
            "/",
            "string.regexp.js punctuation.definition.string.begin.js",
        );
        has(&m, "^", "string.regexp.js keyword.control.anchor.regexp");
        has(
            &m,
            "[",
            "string.regexp.js constant.other.character-class.set.regexp punctuation.definition.character-class.begin.regexp",
        );
        // a `.` in a class is a character, the class's text as the `/` is
        has(
            &m,
            "/.",
            "string.regexp.js constant.other.character-class.set.regexp",
        );
        has(
            &m,
            r"\/",
            "string.regexp.js constant.character.escape.backslash.regexp",
        );
        has(&m, "n", "string.regexp.js entity.name.other.group.regexp");
        has(&m, "n", "string.regexp.js variable.other.group.regexp");
        has(
            &m,
            "2",
            "string.regexp.js keyword.operator.quantifier.regexp constant.literal.numeric.regexp",
        );
        has(
            &m,
            r"\u{1F600}",
            "string.regexp.js constant.character.escape.backslash.regexp",
        );
        has(
            &m,
            "L",
            "string.regexp.js support.constant.unicode-property.regexp",
        );
        // not Java's anchor
        has(
            &m,
            r"\A",
            "string.regexp.js constant.character.escape.backslash.regexp",
        );
        has(&m, "gsu", "string.regexp.js keyword.other.js");
        has(&m, ";", "punctuation.terminator.statement.js");
        // Java's constructs are none of ECMAScript's
        let m = js(&[r"x = /(?i)a++[a-z&&[^q]]/;"]);
        assert!(!m.iter().any(|(t, _)| *t == "?i" || *t == "&&"), "{m:?}");
        // an open group carries to the next line, as bat's contexts do
        let m = js(&["x = /(a/;", "b)/g; y"]);
        has(
            &m,
            ")",
            "string.regexp.js punctuation.definition.group.end.regexp",
        );
        has(&m, "g", "string.regexp.js keyword.other.js");
    }

    #[test]
    fn jsx_closes_across_lines() {
        let m = js(&[
            "const a = (",
            "  <div className=\"x\">",
            "    {b}",
            "  </div>",
            ");",
        ]);
        assert!(
            m.contains(&("div", "meta.tag.jsx entity.name.tag.jsx")),
            "{m:?}"
        );
        assert!(
            m.contains(&("className", "entity.other.attribute-name.jsx")),
            "{m:?}"
        );
        assert!(
            m.contains(&(";", "punctuation.terminator.statement.js")),
            "{m:?}"
        );
    }

    /// `RegExp(…)`'s first argument, a string or a template, is a regex (as WebStorm colors it):
    /// the string's escapes, end and `${…}` come first, and a group open over a `${…}` or a
    /// line's end is kept.
    #[test]
    fn regexes_in_regexp_strings() {
        let has = |m: &[(&str, &str)], text: &str, scopes: &str| {
            assert!(m.contains(&(text, scopes)), "{text:?} {scopes:?} in {m:?}");
        };
        const Q: &str = "string.quoted.js";
        const T: &str = "string.interpolated.js";
        let m = js(&[
            r#"r = new RegExp("^(\\d+)\"x\d$", 'g'), s = RegExp('a\'(');"#,
            r"t = new RegExp(`^(${p}|x)[0-9]\${q}`);",
            "u = foo(\"(a)\"), v = RegExp(`(a",
            "b)c`), w = RegExp(\"[\\",
            "]\");",
        ]);
        has(&m, "^", &format!("{Q} keyword.control.anchor.regexp"));
        has(
            &m,
            r"\\d",
            &format!("{Q} constant.other.character-class.escape.backslash.regexp"),
        );
        has(&m, "\\\"", &format!("{Q} constant.character.escape.js"));
        // `\d` is the string's `d`, no escape of Babel's
        has(&m, "x\\d", Q);
        has(
            &m,
            "(",
            &format!("{Q} punctuation.definition.group.begin.regexp"),
        );
        has(&m, "g", Q);
        has(&m, "|", &format!("{T} keyword.operator.or.regexp"));
        // `\${` is a `\` and a substitution to Babel: the regex's `\` before it
        has(
            &m,
            "\\",
            &format!("{T} constant.character.escape.backslash.regexp"),
        );
        has(&m, "${", "keyword.other.substitution.begin.js");
        has(&m, "(a)", Q);
        has(
            &m,
            ")",
            &format!("{T} punctuation.definition.group.end.regexp"),
        );
        // a `\` ending the line is the string's continuation, not the regex's
        has(
            &m,
            "\\",
            &format!("{Q} constant.character.escape.newline.js"),
        );
        has(
            &m,
            "]",
            &format!(
                "{Q} constant.other.character-class.set.regexp punctuation.definition.character-class.end.regexp"
            ),
        );
    }
}
