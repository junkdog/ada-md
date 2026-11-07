//! HTML by a scanner of ada's own (ADR-40, speed first), emitting bat's scopes, with bat's `HTML`
//! grammar (Packages at `759d6ee`, 106 contexts as syntect compiles it) as the reference it is
//! checked against. Each context is an arm of one `match` on the top of a stack, finding the
//! leftmost place where one of its patterns matches, in the grammar's order there.
//!
//! **Embeds.** `<style>`'s text is CSS and `style="…"`'s is CSS's `rule-list-body`; `<script>`'s
//! text and `on…="…"`'s are JavaScript. Each runs on that language's scanner, entered at the
//! embed's place on the whole line, so its lookaheads and lookbehinds see what syntect's do. syntect
//! makes an embed's `escape` a zero-width pop tried before the embedded grammar's patterns at every
//! place (`(?=</style)`, `(?="`), so a search that reaches the escape's place before any embedded
//! pattern matches ends the embed there: the embedded spans are cut at the first escape after the
//! embed's start. A token that started before it is a single match and runs past it (JavaScript's
//! `//` comment takes a `</script>` after it, and the line); bytes of a context's text are cut. The
//! scanner sees the line up to [`WINDOW`] bytes past the escape, so a line of many `style="…"`s
//! stays linear. syntect's loop guard is followed where it shows: `</style >` (the escape, but not
//! `style-close-tag`) leaves only the embed's escape context, to the next `</style` ([`STYLE_ESC`]).
//!
//! **Kinds.** A kind of HTML's own is a leaf, bat's scopes for the token, under the string, comment
//! or name it sits in ([`LEAVES`]); meta scopes are left out (no theme selects one). An embedded
//! token is the embedded language's kind with [`CSS_KIND`] or [`JS_KIND`] set, under `source.css` or
//! `source.js`; the bytes between embedded tokens stay plain (no theme selects `source.css`).
//!
//! **State.** HTML's stack above `main`, up to 28 contexts, in [`State`]'s levels; or, when a line
//! ends inside an embed, the embedded language's state there and HTML's stack below it in
//! `State::arg` ([`frame`]): `main`, the `<script type="text/html">` bodies open around it, and the
//! embed's host (a `<style>`'s or `<script>`'s body, or an attribute's value and its tag).
//!
//! `(?i)` folds ASCII case only, `\s` and `\S` are Unicode's, `\b`'s word characters are ASCII's
//! and, past ASCII, letters and numbers.

use crate::{Out, Span, State};

// ---- the contexts ----

const MAIN: u8 = 0;
/// `<!-- … -->`.
const COMMENT: u8 = 1;
/// `<![CDATA[ … ]]>`.
const CDATA: u8 = 2;
/// `doctype-meta`: to its `>`.
const DT_META: u8 = 3;
/// `doctype-content`: `[`, a string, or a pop.
const DT_CONTENT: u8 = 4;
/// `doctype-content-type`: `PUBLIC`, `SYSTEM`, or a pop.
const DT_TYPE: u8 = 5;
/// `doctype-name`: a name, or a pop.
const DT_NAME: u8 = 6;
/// A doctype's internal subset, `[ … ]`.
const DT_SUBSET: u8 = 7;
/// `<?xml … ?>`.
const PI: u8 = 8;
/// `"…"`: `string-double-quoted`, and an attribute's value (`tag-generic-attribute-value`'s, the
/// class's and the id's: the same leaves).
const STR_D: u8 = 9;
/// `'…'`, as [`STR_D`].
const STR_S: u8 = 10;
/// A tag that ends at `>`: `tag-end` and `tag-attributes` (structure, block and form tags, a
/// `</script` closing tag).
const TAG: u8 = 11;
/// A tag that ends at `/?>`: `tag-end-maybe-self-closing` and `tag-attributes` (`hr`, inline,
/// form, `a`, table, custom and other tags).
const TAG_SC: u8 = 12;
/// `tag-custom-name`.
const CUSTOM_NAME: u8 = 13;
/// `tag-other-name`.
const OTHER_NAME: u8 = 14;
/// An attribute's `meta`: `immediately-pop`.
const META: u8 = 15;
/// An attribute's `=` or a pop (`tag-generic-attribute-equals`, the class's, the id's).
const EQ: u8 = 16;
/// `tag-style-attribute-equals`.
const EQ_STYLE: u8 = 17;
/// `tag-event-attribute-equals`.
const EQ_EVENT: u8 = 18;
/// `tag-generic-attribute-name`.
const ATTR_NAME: u8 = 19;
/// An attribute's value (`tag-generic-attribute-value`, the class's, the id's).
const VALUE: u8 = 20;
/// An unquoted attribute value.
const UNQUOTED: u8 = 21;
/// `tag-style-attribute-value`: its quote embeds CSS.
const STYLE_VALUE: u8 = 22;
/// `tag-event-attribute-value`: its quote embeds JavaScript.
const EVENT_VALUE: u8 = 23;
/// `<style`'s tag: `style-css`.
const STYLE_TAG: u8 = 24;
/// `style-other`: a `<style>` whose type isn't CSS.
const STYLE_OTHER: u8 = 25;
/// After `<style`'s `type`: its `=` or back to [`STYLE_TAG`].
const STYLE_TYPE: u8 = 26;
/// `style-type-decider`.
const STYLE_TYPE_EQ: u8 = 27;
/// `<style>`'s body: `style-close-tag`, then CSS.
const STYLE_BODY: u8 = 28;
/// `style-other`'s body: `style-close-tag` alone.
const STYLE_OTHER_BODY: u8 = 29;
/// CSS's embed's escape context alone, to its escape (syntect's loop guard).
const STYLE_ESC: u8 = 30;
/// `<script`'s tag: `script-javascript`.
const SCRIPT_TAG: u8 = 31;
/// `script-html`: a `<script type="text/html">`.
const SCRIPT_HTML: u8 = 32;
/// `script-other`: a `<script>` of another type.
const SCRIPT_OTHER: u8 = 33;
/// After `<script`'s `type`: its `=` or back to [`SCRIPT_TAG`].
const SCRIPT_TYPE: u8 = 34;
/// `script-type-decider`.
const SCRIPT_TYPE_EQ: u8 = 35;
/// `<script>`'s body: `script-close-tag`, then JavaScript.
const SCRIPT_BODY: u8 = 36;
/// `script-html`'s body: a comment, `script-close-tag`, `main`.
const SCRIPT_HTML_BODY: u8 = 37;
/// `script-other`'s body: `script-close-tag` alone.
const SCRIPT_OTHER_BODY: u8 = 38;
/// `script-close-tag`'s set: `-->`, then `</script`.
const SCRIPT_CLOSE: u8 = 39;

// ---- the kinds ----

macro_rules! leaves {
    ($($name:ident = $scopes:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u16)]
        enum Leaf { $($name,)* }
        $(#[allow(dead_code)] const $name: u16 = Leaf::$name as u16;)*
        /// Each kind's scopes under `text.html.basic`, outermost first.
        static LEAVES: &[&str] = &[$($scopes,)*];
    };
}

leaves! {
    PLAIN = "",
    TAG_BEGIN = "punctuation.definition.tag.begin.html",
    TAG_END = "punctuation.definition.tag.end.html",
    N_STRUCTURE = "entity.name.tag.structure.any.html",
    N_BLOCK = "entity.name.tag.block.any.html",
    N_BLOCK_FORM = "entity.name.tag.block.form.html",
    N_INLINE = "entity.name.tag.inline.any.html",
    N_INLINE_FORM = "entity.name.tag.inline.form.html",
    N_A = "entity.name.tag.inline.a.html",
    N_TABLE = "entity.name.tag.inline.table.html",
    N_STYLE = "entity.name.tag.style.html",
    N_SCRIPT = "entity.name.tag.script.html",
    N_XML = "entity.name.tag.xml.html",
    N_CUSTOM = "entity.name.tag.custom.html",
    N_CUSTOM_BAD = "entity.name.tag.custom.html invalid.illegal.custom-tag-name.html",
    N_OTHER = "entity.name.tag.other.html",
    ATTR = "entity.other.attribute-name.html",
    ATTR_BAD = "entity.other.attribute-name.html invalid.illegal.attribute-name.html",
    ATTR_ID = "entity.other.attribute-name.id.html",
    ATTR_CLASS = "entity.other.attribute-name.class.html",
    ATTR_STYLE = "entity.other.attribute-name.style.html",
    ATTR_EVENT = "entity.other.attribute-name.event.html",
    KEY_VALUE = "punctuation.separator.key-value.html",
    SD = "string.quoted.double.html",
    SD_BEGIN = "string.quoted.double.html punctuation.definition.string.begin.html",
    SD_END = "string.quoted.double.html punctuation.definition.string.end.html",
    SS = "string.quoted.single.html",
    SS_BEGIN = "string.quoted.single.html punctuation.definition.string.begin.html",
    SS_END = "string.quoted.single.html punctuation.definition.string.end.html",
    UNQ = "string.unquoted.html",
    UNQ_BAD = "string.unquoted.html invalid.illegal.attribute-value.html",
    ED_BEGIN = "string.quoted.double punctuation.definition.string.begin.html",
    ED_END = "string.quoted.double punctuation.definition.string.end.html",
    ES_BEGIN = "string.quoted.single punctuation.definition.string.begin.html",
    ES_END = "string.quoted.single punctuation.definition.string.end.html",
    COMMENT_T = "comment.block.html",
    COMMENT_BEGIN = "comment.block.html punctuation.definition.comment.begin.html",
    COMMENT_END = "comment.block.html punctuation.definition.comment.end.html",
    COMMENT_BAD = "comment.block.html invalid.illegal.bad-comments-or-CDATA.html",
    CDATA_KW = "keyword.declaration.cdata.html",
    CDATA_T = "string.unquoted.cdata.html",
    DOCTYPE_KW = "keyword.declaration.doctype.html",
    DOCTYPE_NAME = "constant.language.doctype.html",
    EXTERNAL = "keyword.content.external.html",
    BRACKETS_BEGIN = "punctuation.section.brackets.begin.html",
    BRACKETS_END = "punctuation.section.brackets.end.html",
    INCOMPLETE = "invalid.illegal.incomplete.html",
    // a character reference: hexadecimal, decimal, named, each its text, its `&` or `&#` and its
    // `;`, in each wrap: none, a double- or single-quoted string, an unquoted value
    E_HEX = "constant.character.entity.hexadecimal.html",
    E_HEX_MARK = "constant.character.entity.hexadecimal.html punctuation.definition.entity.html",
    E_HEX_END = "constant.character.entity.hexadecimal.html punctuation.terminator.entity.html",
    E_DEC = "constant.character.entity.decimal.html",
    E_DEC_MARK = "constant.character.entity.decimal.html punctuation.definition.entity.html",
    E_DEC_END = "constant.character.entity.decimal.html punctuation.terminator.entity.html",
    E_NAMED = "constant.character.entity.named.html",
    E_NAMED_MARK = "constant.character.entity.named.html punctuation.definition.entity.html",
    E_NAMED_END = "constant.character.entity.named.html punctuation.terminator.entity.html",
    SD_HEX = "string.quoted.double.html constant.character.entity.hexadecimal.html",
    SD_HEX_MARK = "string.quoted.double.html constant.character.entity.hexadecimal.html punctuation.definition.entity.html",
    SD_HEX_END = "string.quoted.double.html constant.character.entity.hexadecimal.html punctuation.terminator.entity.html",
    SD_DEC = "string.quoted.double.html constant.character.entity.decimal.html",
    SD_DEC_MARK = "string.quoted.double.html constant.character.entity.decimal.html punctuation.definition.entity.html",
    SD_DEC_END = "string.quoted.double.html constant.character.entity.decimal.html punctuation.terminator.entity.html",
    SD_NAMED = "string.quoted.double.html constant.character.entity.named.html",
    SD_NAMED_MARK = "string.quoted.double.html constant.character.entity.named.html punctuation.definition.entity.html",
    SD_NAMED_END = "string.quoted.double.html constant.character.entity.named.html punctuation.terminator.entity.html",
    SS_HEX = "string.quoted.single.html constant.character.entity.hexadecimal.html",
    SS_HEX_MARK = "string.quoted.single.html constant.character.entity.hexadecimal.html punctuation.definition.entity.html",
    SS_HEX_END = "string.quoted.single.html constant.character.entity.hexadecimal.html punctuation.terminator.entity.html",
    SS_DEC = "string.quoted.single.html constant.character.entity.decimal.html",
    SS_DEC_MARK = "string.quoted.single.html constant.character.entity.decimal.html punctuation.definition.entity.html",
    SS_DEC_END = "string.quoted.single.html constant.character.entity.decimal.html punctuation.terminator.entity.html",
    SS_NAMED = "string.quoted.single.html constant.character.entity.named.html",
    SS_NAMED_MARK = "string.quoted.single.html constant.character.entity.named.html punctuation.definition.entity.html",
    SS_NAMED_END = "string.quoted.single.html constant.character.entity.named.html punctuation.terminator.entity.html",
    UNQ_HEX = "string.unquoted.html constant.character.entity.hexadecimal.html",
    UNQ_HEX_MARK = "string.unquoted.html constant.character.entity.hexadecimal.html punctuation.definition.entity.html",
    UNQ_HEX_END = "string.unquoted.html constant.character.entity.hexadecimal.html punctuation.terminator.entity.html",
    UNQ_DEC = "string.unquoted.html constant.character.entity.decimal.html",
    UNQ_DEC_MARK = "string.unquoted.html constant.character.entity.decimal.html punctuation.definition.entity.html",
    UNQ_DEC_END = "string.unquoted.html constant.character.entity.decimal.html punctuation.terminator.entity.html",
    UNQ_NAMED = "string.unquoted.html constant.character.entity.named.html",
    UNQ_NAMED_MARK = "string.unquoted.html constant.character.entity.named.html punctuation.definition.entity.html",
    UNQ_NAMED_END = "string.unquoted.html constant.character.entity.named.html punctuation.terminator.entity.html",
}

/// A character reference's leaves in a wrap: the wrap's first, then 3 a reference type, then 1
/// its part (text, `&`, `;`).
const E_NONE: u16 = E_HEX;
const E_SD: u16 = SD_HEX;
const E_SS: u16 = SS_HEX;
const E_UNQ: u16 = UNQ_HEX;
const E_DEC_OFF: u16 = 3;
const E_NAMED_OFF: u16 = 6;

/// An embedded CSS token: CSS's kind with this bit set.
pub const CSS_KIND: u16 = 0x1000;
/// An embedded JavaScript token: JavaScript's kind with this bit set.
pub const JS_KIND: u16 = 0x2000;

/// The scopes under `text.html.basic` a kind carries, outermost first.
pub fn scopes(kind: u16) -> Vec<&'static str> {
    match kind & 0xf000 {
        CSS_KIND => {
            let mut v = vec!["source.css"];
            v.extend(crate::css::scopes(kind & 0x0fff));
            v
        }
        JS_KIND => {
            let mut v = vec!["source.js"];
            v.extend(crate::javascript::scopes(kind & 0x0fff));
            v
        }
        0 => LEAVES.get(usize::from(kind)).map_or_else(Vec::new, |l| {
            l.split(' ').filter(|s| !s.is_empty()).collect()
        }),
        _ => Vec::new(),
    }
}

// ---- embeds ----

/// What an embed is: its language and where it sits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Emb {
    /// `<style>`'s text: CSS, to `</style`.
    StyleTag = 1,
    /// `<script>`'s text: JavaScript, to `</script`.
    ScriptTag = 2,
    /// `style="…"`: CSS's `rule-list-body`, to the quote.
    StyleAttr = 3,
    /// `on…="…"`: JavaScript, to the quote.
    EventAttr = 4,
}

impl Emb {
    fn from_bits(v: u16) -> Option<Emb> {
        Some(match v {
            1 => Emb::StyleTag,
            2 => Emb::ScriptTag,
            3 => Emb::StyleAttr,
            4 => Emb::EventAttr,
            _ => return None,
        })
    }

    fn css(self) -> bool {
        matches!(self, Emb::StyleTag | Emb::StyleAttr)
    }

    fn attr(self) -> bool {
        matches!(self, Emb::StyleAttr | Emb::EventAttr)
    }
}

/// How far past an embed's escape its scanner sees the line, for the lookaheads of the tokens
/// before the escape: so a line of many `style="…"`s isn't read again for each. An attribute's
/// sees less: a line of syntect's own HTML output holds a `style="…"` for each token.
const WINDOW: usize = 512;
const WINDOW_ATTR: usize = 64;

/// `State::arg` while a line ends inside an embed: the embed (3 bits from 13), a single quote
/// (bit 12), how many `<script type="text/html">` bodies are open below it (6 bits from 6), and
/// an attribute's tag context (6 bits).
fn frame(emb: Emb, single: bool, bodies: usize, tag: u8) -> u16 {
    ((emb as u16) << 13)
        | (u16::from(single) << 12)
        | ((bodies.min(63) as u16) << 6)
        | u16::from(tag)
}

// ---- the scan ----

/// How many contexts a line can hold; past it the bottom one drops.
const MAX: usize = 128;

/// Lex `b`, a line of HTML without its `\n`, from `state`.
#[inline(never)]
pub fn line(b: &[u8], state: &mut State, out: &mut Out) {
    let mut s = Scan {
        b,
        out,
        stack: [MAIN; MAX],
        depth: 1,
        at: 0,
        exact: state.is_exact(),
        emb: None,
    };
    let p = s.begin(state);
    if p != usize::MAX {
        s.run(p);
    }
    *state = s.state();
}

/// A block of HTML on one scan opened from `state` ([`crate::Carry`], X-067), `state` left as
/// the last line leaves it. A line that starts inside an embed starts from the state, as
/// [`line`]'s does.
pub fn block(
    state: &mut State,
    spans: &mut Vec<crate::Span>,
    pass: &mut dyn FnMut(&mut dyn crate::Carry) -> Option<usize>,
) -> Option<usize> {
    let mut out = Out::new(spans);
    let mut s = Scan {
        b: b"",
        out: &mut out,
        stack: [MAIN; MAX],
        depth: 1,
        at: 0,
        exact: state.is_exact(),
        emb: None,
    };
    // the state loaded, not lexed on an empty line: an embed's language would end what an empty
    // line ends (a CSS string's `\` continuation, X-080); the first line reopens it from here
    s.emb = s.load(state);
    let rest = pass(&mut s);
    *state = s.state();
    rest
}

impl crate::Carry for Scan<'_, '_, '_> {
    fn next(&mut self, b: &[u8]) -> &[crate::Span] {
        self.out.0.clear();
        // SAFETY: the line is set back to an empty one below, before `b`'s borrow ends
        self.b = unsafe { crate::line_for(b) };
        self.at = 0;
        let p = if self.emb.is_some() {
            let state = self.state();
            self.reopen(&state)
        } else {
            0
        };
        if p != usize::MAX {
            self.run(p);
        }
        // past the levels a state holds, the bottom ones dropped; an embed's line is its own
        if self.emb.is_none() {
            crate::stack::hold(&mut self.stack, &mut self.depth, &mut self.exact);
        }
        self.b = b"";
        self.out.0
    }
}

impl Scan<'_, '_, '_> {
    /// The scan opened from `state` again, at the line's start: where its scan starts
    /// ([`Scan::begin`]).
    #[inline(never)]
    fn reopen(&mut self, state: &State) -> usize {
        (self.depth, self.exact, self.emb) = (1, state.is_exact(), None);
        self.begin(state)
    }

    /// The line's stack from `state`: where the scan starts, or `usize::MAX` where an embed
    /// took the whole line.
    #[inline(always)]
    fn begin(&mut self, state: &State) -> usize {
        match self.load(state) {
            Some((emb, quote, st)) => self.embed(emb, quote, 0, Some(st)).unwrap_or(usize::MAX),
            None => 0,
        }
    }

    /// The stack from `state`; when it's inside an embed, HTML's stack below it, and the embed,
    /// its quote and its language's state.
    #[inline(always)]
    fn load(&mut self, state: &State) -> Option<(Emb, u8, State)> {
        let Some(emb) = Emb::from_bits(state.arg >> 13) else {
            self.depth = state.load(&mut self.stack);
            return None;
        };
        // the line starts inside an embed: HTML's stack below it, then the embedded language
        let arg = state.arg;
        let bodies = usize::from((arg >> 6) & 63);
        for _ in 0..bodies {
            self.push(SCRIPT_HTML_BODY);
        }
        let quote = if arg & (1 << 12) != 0 { b'\'' } else { b'"' };
        match emb {
            Emb::StyleTag => self.push(STYLE_BODY),
            Emb::ScriptTag => self.push(SCRIPT_BODY),
            Emb::StyleAttr | Emb::EventAttr => {
                self.push((arg & 63) as u8);
                self.push(META);
                self.push(if emb == Emb::StyleAttr {
                    STYLE_VALUE
                } else {
                    EVENT_VALUE
                });
            }
        }
        let mut st = *state;
        st.arg = 0;
        Some((emb, quote, st))
    }

    /// The state the scan's last line leaves.
    fn state(&self) -> State {
        if let Some((emb, quote, st)) = self.emb {
            // the stack below the embed: `main`, the bodies, the host
            let host = if emb.attr() { 3 } else { 1 };
            let bodies = self.depth - 1 - host;
            let tag = if emb.attr() {
                self.stack[self.depth - 3]
            } else {
                0
            };
            let exact = self.exact && st.is_exact() && bodies < 64;
            let mut state = st;
            state.inexact = !exact;
            state.arg = frame(emb, quote == b'\'', bodies, tag);
            return state;
        }
        State::keep(&self.stack, self.depth, self.exact, 0)
    }
}

struct Scan<'b, 'o, 's> {
    b: &'b [u8],
    out: &'o mut Out<'s>,
    stack: [u8; MAX],
    depth: usize,
    /// Where the spans reach: the text before a token is filled from here.
    at: usize,
    exact: bool,
    /// The embed the line ends in, its quote, and its language's state there.
    emb: Option<(Emb, u8, State)>,
}

/// Whether `c` ends a tag's name: `[^{{tag_name_char}}]`, `[\t\n\f /<>]`.
fn name_break(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\x0c' | b'/' | b'<' | b'>')
}

/// `[\t\n\f =/>]`, `attribute_name_char`: where an attribute's name ends.
fn attr_break(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\x0c' | b'=' | b'/' | b'>')
}

/// The character at `i` of `b` and its length.
fn char_at(b: &[u8], i: usize) -> (char, usize) {
    let c = b[i];
    if c < 0x80 {
        return (char::from(c), 1);
    }
    let len = match c {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    let end = (i + len).min(b.len());
    match std::str::from_utf8(&b[i..end])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(ch) => (ch, len),
        None => ('\u{fffd}', 1),
    }
}

/// The character that ends at `i` of `b`.
fn char_before(b: &[u8], i: usize) -> char {
    let mut s = i - 1;
    while s > 0 && b[s] & 0xc0 == 0x80 && i - s < 4 {
        s -= 1;
    }
    char_at(b, s).0
}

/// `\w`, approximated: ASCII's, and past it letters and numbers.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `on(…)\b`'s events.
const EVENTS: &[&[u8]] = &[
    b"abort",
    b"autocomplete",
    b"autocompleteerror",
    b"auxclick",
    b"blur",
    b"cancel",
    b"canplay",
    b"canplaythrough",
    b"change",
    b"click",
    b"close",
    b"contextmenu",
    b"cuechange",
    b"dblclick",
    b"drag",
    b"dragend",
    b"dragenter",
    b"dragexit",
    b"dragleave",
    b"dragover",
    b"dragstart",
    b"drop",
    b"durationchange",
    b"emptied",
    b"ended",
    b"error",
    b"focus",
    b"input",
    b"invalid",
    b"keydown",
    b"keypress",
    b"keyup",
    b"load",
    b"loadeddata",
    b"loadedmetadata",
    b"loadstart",
    b"mousedown",
    b"mouseenter",
    b"mouseleave",
    b"mousemove",
    b"mouseout",
    b"mouseover",
    b"mouseup",
    b"mousewheel",
    b"pause",
    b"play",
    b"playing",
    b"progress",
    b"ratechange",
    b"reset",
    b"resize",
    b"scroll",
    b"seeked",
    b"seeking",
    b"select",
    b"show",
    b"sort",
    b"stalled",
    b"submit",
    b"suspend",
    b"timeupdate",
    b"toggle",
    b"volumechange",
    b"waiting",
];

/// A tag's name, lowercase ASCII: its name's leaf and whether its tag ends at `/?>`, as the
/// grammar's lists have it (structure, block, `hr`, block form, inline, form, `a`, table, in
/// the order `main` tries them).
fn tag_kind(lower: &[u8]) -> Option<(u16, bool)> {
    Some(match lower {
        b"body" | b"head" | b"html" => (N_STRUCTURE, false),
        b"address" | b"applet" | b"article" | b"aside" | b"blockquote" | b"center" | b"dd"
        | b"dir" | b"div" | b"dl" | b"dt" | b"figcaption" | b"figure" | b"footer" | b"frame"
        | b"frameset" | b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6" | b"header" | b"iframe"
        | b"menu" | b"nav" | b"noframes" | b"object" | b"ol" | b"p" | b"pre" | b"section"
        | b"ul" => (N_BLOCK, false),
        b"hr" => (N_BLOCK, true),
        b"form" | b"fieldset" => (N_BLOCK_FORM, false),
        b"abbr" | b"acronym" | b"area" | b"audio" | b"b" | b"base" | b"basefont" | b"bdi"
        | b"bdo" | b"big" | b"br" | b"canvas" | b"caption" | b"cite" | b"code" | b"del"
        | b"details" | b"dfn" | b"dialog" | b"em" | b"font" | b"i" | b"img" | b"ins"
        | b"isindex" | b"kbd" | b"li" | b"link" | b"map" | b"mark" | b"menuitem" | b"meta"
        | b"noscript" | b"param" | b"picture" | b"q" | b"rp" | b"rt" | b"rtc" | b"ruby" | b"s"
        | b"samp" | b"script" | b"small" | b"source" | b"span" | b"strike" | b"strong"
        | b"style" | b"sub" | b"summary" | b"sup" | b"time" | b"title" | b"track" | b"tt"
        | b"u" | b"var" | b"video" | b"wbr" => (N_INLINE, true),
        b"button" | b"datalist" | b"input" | b"label" | b"legend" | b"meter" | b"optgroup"
        | b"option" | b"output" | b"progress" | b"select" | b"template" | b"textarea" => {
            (N_INLINE_FORM, true)
        }
        b"a" => (N_A, true),
        b"col" | b"colgroup" | b"table" | b"tbody" | b"td" | b"tfoot" | b"th" | b"thead"
        | b"tr" => (N_TABLE, true),
        _ => return None,
    })
}

/// `custom_element_char` at `i` of `b`: its length.
fn custom_char(b: &[u8], i: usize) -> Option<usize> {
    let c = b[i];
    if c < 0x80 {
        return match c {
            b'-' | b'_' | b'a'..=b'z' | b'0'..=b'9' => Some(1),
            // `\\\.`: a backslash and a `.`
            b'\\' if b.get(i + 1) == Some(&b'.') => Some(2),
            _ => None,
        };
    }
    let (ch, len) = char_at(b, i);
    let u = u32::from(ch);
    let ok = u == 0xb7
        || (0xc0..=0xd6).contains(&u)
        || (0xd8..=0xf6).contains(&u)
        || (0xf8..=0x37d).contains(&u)
        || (0x37f..=0x1fff).contains(&u)
        || (0x200c..=0x200d).contains(&u)
        || (0x203f..=0x2040).contains(&u)
        || (0x2070..=0x218f).contains(&u)
        || (0x2c00..=0x2fef).contains(&u)
        || (0x3001..=0xd7ff).contains(&u)
        || (0xf900..=0xfdcf).contains(&u)
        || (0xfdf0..=0xfffd).contains(&u)
        || (0x10000..=0xeffff).contains(&u);
    ok.then_some(len)
}

/// A JavaScript MIME type as `script-type-decider` reads one, lowercase.
const JS_MIMES: &[&[u8]] = &[
    b"application/javascript",
    b"application/x-javascript",
    b"application/ecmascript",
    b"application/x-ecmascript",
    b"text/javascript",
    b"text/x-javascript",
    b"text/ecmascript",
    b"text/x-ecmascript",
    b"text/javascript1.0",
    b"text/javascript1.1",
    b"text/javascript1.2",
    b"text/javascript1.3",
    b"text/javascript1.4",
    b"text/javascript1.5",
    b"text/jscript",
    b"text/livescript",
];

impl Scan<'_, '_, '_> {
    fn top(&self) -> u8 {
        self.stack[self.depth - 1]
    }

    fn push(&mut self, ctx: u8) {
        if self.depth == MAX {
            crate::stack::drop_bottom(&mut self.stack, &mut self.depth, &mut self.exact);
        }
        self.stack[self.depth] = ctx;
        self.depth += 1;
    }

    fn pop(&mut self) {
        if self.depth > 1 {
            self.depth -= 1;
        }
    }

    fn set(&mut self, ctx: u8) {
        self.stack[self.depth - 1] = ctx;
    }

    fn c(&self, i: usize) -> Option<u8> {
        self.b.get(i).copied()
    }

    /// Whether `s` is at `i`.
    fn is(&self, i: usize, s: &[u8]) -> bool {
        self.b.get(i..i + s.len()) == Some(s)
    }

    /// Whether `s`, lowercase, is at `i`, ignoring ASCII case.
    fn is_ic(&self, i: usize, s: &[u8]) -> bool {
        self.b
            .get(i..i + s.len())
            .is_some_and(|t| t.eq_ignore_ascii_case(s))
    }

    /// The text kind of the top context: its meta scope or meta content scope a theme can see.
    fn text(&self) -> u16 {
        match self.top() {
            COMMENT => COMMENT_T,
            CDATA => CDATA_T,
            STR_D => SD,
            STR_S => SS,
            UNQUOTED => UNQ,
            ATTR_NAME => ATTR,
            CUSTOM_NAME => N_CUSTOM,
            OTHER_NAME => N_OTHER,
            _ => PLAIN,
        }
    }

    /// Fill the top context's text up to `to`.
    fn fill(&mut self, to: usize) {
        let to = to.min(self.b.len());
        if to > self.at {
            let t = self.text();
            if t != PLAIN {
                self.out.push_merged(self.at, to, t);
            }
            self.at = to;
        }
    }

    /// Bytes `s..e` are `kind`, after the text before them.
    fn tok(&mut self, s: usize, e: usize, kind: u16) {
        self.fill(s);
        let e = e.min(self.b.len());
        if e > self.at {
            self.out.push_merged(self.at, e, kind);
            self.at = e;
        }
    }

    /// Where the `\s` run at `i` ends.
    fn spaces(&self, mut i: usize) -> usize {
        while i < self.b.len() {
            let c = self.b[i];
            if c < 0x80 {
                if !crate::is_space(c) {
                    break;
                }
                i += 1;
            } else {
                let (ch, len) = char_at(self.b, i);
                if !ch.is_whitespace() {
                    break;
                }
                i += len;
            }
        }
        i
    }

    /// The next character's place after `i`.
    fn next(&self, i: usize) -> usize {
        if i >= self.b.len() {
            return i + 1;
        }
        let mut j = i + 1;
        while j < self.b.len() && self.b[j] & 0xc0 == 0x80 {
            j += 1;
        }
        j
    }

    /// `\b` at `i` before a word character there.
    fn word_start(&self, i: usize) -> bool {
        i == 0 || !is_word(char_before(self.b, i))
    }

    /// Whether a word character is at `i` (not the line's `\n`).
    fn word_at(&self, i: usize) -> bool {
        i < self.b.len() && is_word(char_at(self.b, i).0)
    }

    /// Where a tag's name that starts at `i` ends: `{{tag_name_char}}*`.
    fn name_end(&self, mut i: usize) -> usize {
        while i < self.b.len() && !name_break(self.b[i]) {
            i += 1;
        }
        i
    }

    /// `{{unquoted_attribute_break}}` at `i`: `(?=[\t\n\f ]|/?>)`.
    fn unquoted_break(&self, i: usize) -> bool {
        match self.c(i) {
            None => true,
            Some(b' ' | b'\t' | b'\x0c' | b'>') => true,
            Some(b'/') => self.c(i + 1) == Some(b'>'),
            _ => false,
        }
    }

    /// One of `names` at `i`, ignoring ASCII case: unquoted before an unquoted value's end, or
    /// whole in quotes of either kind.
    fn value_is(&self, i: usize, names: &[&[u8]]) -> bool {
        names.iter().any(|name| {
            (self.is_ic(i, name) && self.unquoted_break(i + name.len()))
                || matches!(self.c(i), Some(q @ (b'"' | b'\''))
                    if self.is_ic(i + 1, name) && self.c(i + 1 + name.len()) == Some(q))
        })
    }

    /// `(?i)(?=(?:-->\s*)?</TAG)` at `i`, `tag` lowercase.
    fn close_at(&self, i: usize, tag: &[u8]) -> bool {
        let closing = |j: usize| self.is(j, b"</") && self.is_ic(j + 2, tag);
        closing(i) || (self.is(i, b"-->") && closing(self.spaces(i + 3)))
    }

    /// The first place from `i` where `(?i)(?=(?:-->\s*)?</TAG)` matches.
    fn find_close(&self, mut i: usize, tag: &[u8]) -> Option<usize> {
        while i < self.b.len() {
            let c = self.b[i];
            if (c == b'<' || c == b'-') && self.close_at(i, tag) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// A character reference at `i` in `wrap` ([`E_NONE`] and the like): its tokens, and where it
    /// ends.
    fn entity(&mut self, i: usize, wrap: u16) -> Option<usize> {
        let (b, n) = (self.b, self.b.len());
        let run = |mut j: usize, f: fn(&u8) -> bool| {
            while j < n && f(&b[j]) {
                j += 1;
            }
            j
        };
        let mut hit = None;
        if self.c(i + 1) == Some(b'#') {
            if matches!(self.c(i + 2), Some(b'x' | b'X')) {
                let e = run(i + 3, u8::is_ascii_hexdigit);
                if e > i + 3 && self.c(e) == Some(b';') {
                    hit = Some((i + 3, e, 0));
                }
            }
            if hit.is_none() {
                let e = run(i + 2, u8::is_ascii_digit);
                if e > i + 2 && self.c(e) == Some(b';') {
                    hit = Some((i + 2, e, E_DEC_OFF));
                }
            }
        }
        if hit.is_none() {
            let e = run(i + 1, u8::is_ascii_alphanumeric);
            if e > i + 1 && self.c(e) == Some(b';') {
                hit = Some((i + 1, e, E_NAMED_OFF));
            }
        }
        let (m, e, off) = hit?;
        let base = wrap + off;
        self.tok(i, m, base + 1);
        self.tok(m, e, base);
        self.tok(e, e + 1, base + 2);
        Some(e + 1)
    }

    // ---- the run ----

    /// The line from `p`: each context finds its next match and applies it.
    fn run(&mut self, mut p: usize) {
        let mut guard = 0u32;
        loop {
            let r = match self.top() {
                MAIN | SCRIPT_HTML_BODY => self.main(p),
                COMMENT => self.comment(p),
                CDATA => self.cdata(p),
                DT_META => self.dt_meta(p),
                DT_CONTENT => self.dt_content(p),
                DT_TYPE => self.dt_type(p),
                DT_NAME => self.dt_name(p),
                DT_SUBSET => self.dt_subset(p),
                PI => self.pi(p),
                STR_D | STR_S => self.string(p),
                TAG | TAG_SC | STYLE_TAG | STYLE_OTHER | SCRIPT_TAG | SCRIPT_HTML
                | SCRIPT_OTHER => self.tag(p),
                CUSTOM_NAME => self.custom_name(p),
                OTHER_NAME => self.other_name(p),
                META => {
                    // `immediately-pop`
                    self.pop();
                    Some(p)
                }
                EQ | EQ_STYLE | EQ_EVENT => self.equals(p),
                ATTR_NAME => self.attr_name(p),
                VALUE => self.value(p),
                UNQUOTED => self.unquoted(p),
                STYLE_VALUE | EVENT_VALUE => self.embed_value(p),
                STYLE_TYPE | SCRIPT_TYPE => self.type_attr(p),
                STYLE_TYPE_EQ | SCRIPT_TYPE_EQ => self.type_value(p),
                STYLE_BODY => self.style_body(p),
                STYLE_OTHER_BODY => self.style_other_body(p),
                STYLE_ESC => self.style_esc(p),
                SCRIPT_BODY => self.script_body(p),
                SCRIPT_OTHER_BODY => self.script_other_body(p),
                SCRIPT_CLOSE => self.script_close(p),
                _ => None,
            };
            match r {
                Some(q) => {
                    if q == p {
                        guard += 1;
                        if guard > 4 * MAX as u32 {
                            // a push and a pop at one place forever: step a character
                            guard = 0;
                            if p >= self.b.len() {
                                break;
                            }
                            p = self.next(p);
                            continue;
                        }
                    } else {
                        guard = 0;
                    }
                    p = q;
                }
                None => break,
            }
        }
        if self.emb.is_none() {
            self.fill(self.b.len());
        }
    }

    /// `main`, and `script-html`'s body (`main` after a comment and `script-close-tag`).
    fn main(&mut self, mut p: usize) -> Option<usize> {
        let shb = self.top() == SCRIPT_HTML_BODY;
        while p < self.b.len() {
            match self.b[p] {
                b'<' => {
                    if let Some(q) = self.main_lt(p, shb) {
                        return Some(q);
                    }
                }
                b'&' => {
                    if let Some(q) = self.entity(p, E_NONE) {
                        return Some(q);
                    }
                }
                b'-' if shb && self.close_at(p, b"script") => {
                    self.fill(p);
                    self.set(SCRIPT_CLOSE);
                    return Some(p);
                }
                _ => {}
            }
            p += 1;
        }
        None
    }

    /// `<!--`'s push, from `comment`: its tokens and the comment's context.
    fn open_comment(&mut self, p: usize) -> usize {
        self.tok(p, p + 4, COMMENT_BEGIN);
        let mut e = p + 4;
        if self.c(e) == Some(b'>') {
            e += 1;
        } else if self.is(e, b"->") {
            e += 2;
        }
        self.push(COMMENT);
        self.tok(p + 4, e, COMMENT_BAD);
        e
    }

    /// `main`'s patterns at a `<`.
    fn main_lt(&mut self, p: usize, shb: bool) -> Option<usize> {
        if self.is(p, b"<!--") {
            return Some(self.open_comment(p));
        }
        if self.is(p, b"<![CDATA[") {
            self.tok(p, p + 3, TAG_BEGIN);
            self.tok(p + 3, p + 8, CDATA_KW);
            self.tok(p + 8, p + 9, TAG_BEGIN);
            self.push(CDATA);
            return Some(p + 9);
        }
        if self.is(p, b"<!DOCTYPE") {
            self.tok(p, p + 2, TAG_BEGIN);
            self.tok(p + 2, p + 9, DOCTYPE_KW);
            self.push(DT_META);
            self.push(DT_CONTENT);
            self.push(DT_TYPE);
            self.push(DT_NAME);
            return Some(p + 9);
        }
        if self.is(p, b"<?xml") {
            self.tok(p, p + 2, TAG_BEGIN);
            self.tok(p + 2, p + 5, N_XML);
            self.push(PI);
            return Some(p + 5);
        }
        if shb && self.close_at(p, b"script") {
            self.fill(p);
            self.set(SCRIPT_CLOSE);
            return Some(p);
        }
        let close = self.c(p + 1) == Some(b'/');
        let s = p + 1 + usize::from(close);
        if self.c(s).is_some_and(|c| c.is_ascii_alphabetic()) {
            let e = self.name_end(s);
            let name = &self.b[s..e];
            let mut lower = [0u8; 16];
            let known = if name.len() <= lower.len() {
                lower[..name.len()].copy_from_slice(name);
                lower[..name.len()].make_ascii_lowercase();
                Some(&lower[..name.len()])
            } else {
                None
            };
            if !close && known == Some(b"style") {
                self.tok(p, p + 1, TAG_BEGIN);
                self.tok(s, e, N_STYLE);
                self.push(STYLE_TAG);
                return Some(e);
            }
            if !close && known == Some(b"script") {
                self.tok(p, p + 1, TAG_BEGIN);
                self.tok(s, e, N_SCRIPT);
                self.push(SCRIPT_TAG);
                return Some(e);
            }
            if let Some((leaf, sc)) = known.and_then(tag_kind) {
                self.tok(p, s, TAG_BEGIN);
                self.tok(s, e, leaf);
                self.push(if sc { TAG_SC } else { TAG });
                return Some(e);
            }
            let custom = name[1..].contains(&b'-');
            self.tok(p, s, TAG_BEGIN);
            self.push(TAG_SC);
            self.push(if custom { CUSTOM_NAME } else { OTHER_NAME });
            return Some(s);
        }
        if self.c(p + 1) == Some(b'>') {
            self.tok(p, p + 2, INCOMPLETE);
            return Some(p + 2);
        }
        None
    }

    /// A comment's body: `(<!-)?(--\s*>)`, a pop, and `<!--(?!-?>)|--!>`.
    fn comment(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            match self.b[p] {
                b'<' => {
                    if self.is(p, b"<!-")
                        && let Some(e) = self.comment_end(p + 3)
                    {
                        self.tok(p, p + 3, COMMENT_BAD);
                        self.tok(p + 3, e, COMMENT_END);
                        self.pop();
                        return Some(e);
                    }
                    if self.is(p, b"<!--") && !self.is(p + 4, b">") && !self.is(p + 4, b"->") {
                        self.tok(p, p + 4, COMMENT_BAD);
                        p += 4;
                        continue;
                    }
                }
                b'-' => {
                    if let Some(e) = self.comment_end(p) {
                        self.tok(p, e, COMMENT_END);
                        self.pop();
                        return Some(e);
                    }
                    if self.is(p, b"--!>") {
                        self.tok(p, p + 4, COMMENT_BAD);
                        p += 4;
                        continue;
                    }
                }
                _ => {}
            }
            p += 1;
        }
        None
    }

    /// `--\s*>` at `i`: where it ends.
    fn comment_end(&self, i: usize) -> Option<usize> {
        if !self.is(i, b"--") {
            return None;
        }
        let s = self.spaces(i + 2);
        (self.c(s) == Some(b'>')).then_some(s + 1)
    }

    fn cdata(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            if self.b[p] == b']' && self.is(p, b"]]>") {
                self.tok(p, p + 3, TAG_END);
                self.pop();
                return Some(p + 3);
            }
            p += 1;
        }
        None
    }

    fn dt_meta(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            if self.b[p] == b'>' {
                self.tok(p, p + 1, TAG_END);
                self.pop();
                return Some(p + 1);
            }
            p += 1;
        }
        None
    }

    fn dt_content(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let c = self.c(q)?;
        match c {
            b'[' => {
                self.tok(q, q + 1, BRACKETS_BEGIN);
                self.set(DT_SUBSET);
                Some(q + 1)
            }
            b'"' => {
                self.tok(q, q + 1, SD_BEGIN);
                self.push(STR_D);
                Some(q + 1)
            }
            b'\'' => {
                self.tok(q, q + 1, SS_BEGIN);
                self.push(STR_S);
                Some(q + 1)
            }
            _ => {
                self.fill(q);
                self.pop();
                Some(q)
            }
        }
    }

    fn dt_type(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        self.c(q)?;
        if (self.is(q, b"PUBLIC") || self.is(q, b"SYSTEM")) && !self.word_at(q + 6) {
            self.tok(q, q + 6, EXTERNAL);
            self.pop();
            return Some(q + 6);
        }
        self.fill(q);
        self.pop();
        Some(q)
    }

    fn dt_name(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let c = self.c(q)?;
        if c.is_ascii_alphabetic() {
            let e = self.name_end(q);
            self.tok(q, e, DOCTYPE_NAME);
            self.pop();
            return Some(e);
        }
        self.fill(q);
        self.pop();
        Some(q)
    }

    fn dt_subset(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            match self.b[p] {
                b']' => {
                    self.tok(p, p + 1, BRACKETS_END);
                    self.pop();
                    return Some(p + 1);
                }
                b'<' if self.is(p, b"<!--") => return Some(self.open_comment(p)),
                _ => {}
            }
            p += 1;
        }
        None
    }

    /// `<?xml`'s body: `?>`, then a generic attribute (which takes a string's quote first).
    fn pi(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            let c = self.b[p];
            if c == b'?' && self.c(p + 1) == Some(b'>') {
                self.tok(p, p + 2, TAG_END);
                self.pop();
                return Some(p + 2);
            }
            if !attr_break(c) {
                return Some(self.generic_attr(p));
            }
            p += 1;
        }
        None
    }

    /// `tag-generic-attribute`'s zero-width push at `p`.
    fn generic_attr(&mut self, p: usize) -> usize {
        self.fill(p);
        self.push(META);
        self.push(EQ);
        self.push(ATTR_NAME);
        p
    }

    /// A string: its quote, a pop, or a character reference.
    fn string(&mut self, mut p: usize) -> Option<usize> {
        let (quote, end, wrap) = if self.top() == STR_D {
            (b'"', SD_END, E_SD)
        } else {
            (b'\'', SS_END, E_SS)
        };
        while p < self.b.len() {
            let c = self.b[p];
            if c == quote {
                self.tok(p, p + 1, end);
                self.pop();
                return Some(p + 1);
            }
            if c == b'&'
                && let Some(e) = self.entity(p, wrap)
            {
                p = e;
                continue;
            }
            p += 1;
        }
        None
    }

    /// A tag's body: (`<style`'s and `<script`'s `type`), the attributes, its end.
    fn tag(&mut self, mut p: usize) -> Option<usize> {
        let ctx = self.top();
        let typed = matches!(
            ctx,
            STYLE_TAG | STYLE_OTHER | SCRIPT_TAG | SCRIPT_HTML | SCRIPT_OTHER
        );
        while p < self.b.len() {
            let c = self.b[p];
            if typed
                && matches!(c, b't' | b'T')
                && self.word_start(p)
                && self.is_ic(p, b"type")
                && !self.word_at(p + 4)
            {
                self.tok(p, p + 4, ATTR);
                self.set(if matches!(ctx, STYLE_TAG | STYLE_OTHER) {
                    STYLE_TYPE
                } else {
                    SCRIPT_TYPE
                });
                return Some(p + 4);
            }
            match c {
                b'>' => {
                    self.tok(p, p + 1, TAG_END);
                    match ctx {
                        STYLE_TAG => self.set(STYLE_BODY),
                        STYLE_OTHER => self.set(STYLE_OTHER_BODY),
                        SCRIPT_TAG => self.set(SCRIPT_BODY),
                        SCRIPT_HTML => self.set(SCRIPT_HTML_BODY),
                        SCRIPT_OTHER => self.set(SCRIPT_OTHER_BODY),
                        _ => self.pop(),
                    }
                    return Some(p + 1);
                }
                b'/' if ctx != TAG && self.c(p + 1) == Some(b'>') => {
                    self.tok(p, p + 2, TAG_END);
                    self.pop();
                    return Some(p + 2);
                }
                b' ' | b'\t' | b'\x0c' | b'=' | b'/' => p += 1,
                _ => return Some(self.attribute(p)),
            }
        }
        None
    }

    /// `tag-attributes` at `p`, where an attribute's name can start.
    fn attribute(&mut self, p: usize) -> usize {
        if self.word_start(p) {
            let known = match self.b[p] {
                b'i' if self.is(p, b"id") => Some((2, ATTR_ID, EQ)),
                b'c' if self.is(p, b"class") => Some((5, ATTR_CLASS, EQ)),
                b's' if self.is(p, b"style") => Some((5, ATTR_STYLE, EQ_STYLE)),
                b'o' if self.is(p, b"on") => {
                    let mut e = p + 2;
                    while e < self.b.len() && self.b[e].is_ascii_lowercase() {
                        e += 1;
                    }
                    let name = &self.b[p + 2..e];
                    EVENTS
                        .contains(&name)
                        .then_some((e - p, ATTR_EVENT, EQ_EVENT))
                }
                _ => None,
            };
            if let Some((len, leaf, eq)) = known
                && !self.word_at(p + len)
            {
                self.tok(p, p + len, leaf);
                self.push(META);
                self.push(eq);
                return p + len;
            }
        }
        self.generic_attr(p)
    }

    /// `tag-custom-name`: its characters, others invalid, to the name's end.
    fn custom_name(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            if name_break(self.b[p]) {
                break;
            }
            if let Some(len) = custom_char(self.b, p) {
                p += len;
            } else {
                let e = self.next(p);
                self.tok(p, e, N_CUSTOM_BAD);
                p = e;
            }
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    fn other_name(&mut self, p: usize) -> Option<usize> {
        let e = self.name_end(p);
        self.fill(e);
        self.pop();
        Some(e)
    }

    /// An attribute's name: `["'`<]` invalid, to `[\t\n\f =/>]`.
    fn attr_name(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            match self.b[p] {
                c if attr_break(c) => break,
                b'"' | b'\'' | b'`' | b'<' => {
                    self.tok(p, p + 1, ATTR_BAD);
                }
                _ => {}
            }
            p += 1;
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    /// An attribute's `=`, or a pop before anything else that isn't a space.
    fn equals(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let c = self.c(q)?;
        if c == b'=' {
            self.tok(q, q + 1, KEY_VALUE);
            let value = match self.top() {
                EQ_STYLE => STYLE_VALUE,
                EQ_EVENT => EVENT_VALUE,
                _ => VALUE,
            };
            self.set(value);
            return Some(q + 1);
        }
        self.fill(q);
        self.pop();
        Some(q)
    }

    /// An attribute's value: a quote, an unquoted value, or a pop.
    fn value(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            match self.b[p] {
                b'"' => {
                    self.tok(p, p + 1, SD_BEGIN);
                    self.set(STR_D);
                    return Some(p + 1);
                }
                b'\'' => {
                    self.tok(p, p + 1, SS_BEGIN);
                    self.set(STR_S);
                    return Some(p + 1);
                }
                b'=' | b'>' => {
                    self.fill(p);
                    self.pop();
                    return Some(p);
                }
                b' ' | b'\t' | b'\x0c' => p += 1,
                _ => {
                    self.fill(p);
                    self.set(UNQUOTED);
                    return Some(p);
                }
            }
        }
        None
    }

    /// An unquoted value: to `[\t\n\f ]` or `/?>`, `["'`<]` invalid, character references.
    fn unquoted(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            match self.b[p] {
                b' ' | b'\t' | b'\x0c' | b'>' => break,
                b'/' if self.c(p + 1) == Some(b'>') => break,
                b'"' | b'\'' | b'`' | b'<' => self.tok(p, p + 1, UNQ_BAD),
                b'&' => {
                    if let Some(e) = self.entity(p, E_UNQ) {
                        p = e;
                        continue;
                    }
                }
                _ => {}
            }
            p += 1;
        }
        self.fill(p);
        self.pop();
        Some(p)
    }

    /// `style=`'s and `on…=`'s value: a quote embeds CSS or JavaScript, else a pop.
    fn embed_value(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let c = self.c(q)?;
        let emb = if self.top() == STYLE_VALUE {
            Emb::StyleAttr
        } else {
            Emb::EventAttr
        };
        match c {
            b'"' => {
                self.tok(q, q + 1, ED_BEGIN);
                self.embed(emb, b'"', q + 1, None)
            }
            b'\'' => {
                self.tok(q, q + 1, ES_BEGIN);
                self.embed(emb, b'\'', q + 1, None)
            }
            _ => {
                self.fill(q);
                self.pop();
                Some(q)
            }
        }
    }

    /// After `<style`'s or `<script`'s `type`: its `=`, or back to the tag.
    fn type_attr(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        let c = self.c(q)?;
        let style = self.top() == STYLE_TYPE;
        if c == b'=' {
            self.tok(q, q + 1, KEY_VALUE);
            self.set(if style { STYLE_TYPE_EQ } else { SCRIPT_TYPE_EQ });
            return Some(q + 1);
        }
        self.fill(q);
        self.set(if style { STYLE_TAG } else { SCRIPT_TAG });
        Some(q)
    }

    /// `style-type-decider` and `script-type-decider`: the tag the type's value makes it, with
    /// the value on top.
    fn type_value(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        self.c(q)?;
        let empty = self.is(q, b">") || self.is(q, b"''") || self.is(q, b"\"\"");
        let tag = if self.top() == STYLE_TYPE_EQ {
            if self.value_is(q, &[b"text/css"]) || empty {
                STYLE_TAG
            } else {
                STYLE_OTHER
            }
        } else if self.value_is(q, JS_MIMES) || self.value_is(q, &[b"module"]) || empty {
            SCRIPT_TAG
        } else if self.value_is(q, &[b"text/html"]) {
            SCRIPT_HTML
        } else {
            SCRIPT_OTHER
        };
        self.fill(q);
        self.set(tag);
        self.push(META);
        self.push(VALUE);
        Some(q)
    }

    /// `<style>`'s body: `</style>`, else CSS from the first character that isn't a space.
    fn style_body(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        self.c(q)?;
        if self.is_ic(q, b"</style>") {
            self.style_close(q);
            return Some(q + 8);
        }
        if self.close_at(q, b"style") {
            // the embed's escape at its own push: the escape context alone stays, a character on
            self.fill(q);
            self.push(STYLE_ESC);
            return Some(self.next(q));
        }
        self.embed(Emb::StyleTag, 0, q, None)
    }

    /// `(?i)(</)(style)(>)` at `q`, a pop.
    fn style_close(&mut self, q: usize) {
        self.tok(q, q + 2, TAG_BEGIN);
        self.tok(q + 2, q + 7, N_STYLE);
        self.tok(q + 7, q + 8, TAG_END);
        self.pop();
    }

    fn style_other_body(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            if self.b[p] == b'<' && self.is_ic(p, b"</style>") {
                self.style_close(p);
                return Some(p + 8);
            }
            p += 1;
        }
        None
    }

    fn style_esc(&mut self, p: usize) -> Option<usize> {
        let e = self.find_close(p, b"style")?;
        self.fill(e);
        self.pop();
        Some(e)
    }

    /// `<script>`'s body: `<!--`, `script-close-tag`'s lookahead, else JavaScript.
    fn script_body(&mut self, p: usize) -> Option<usize> {
        let q = self.spaces(p);
        self.c(q)?;
        if self.is(q, b"<!--") {
            self.tok(q, q + 4, COMMENT_BEGIN);
            return Some(q + 4);
        }
        if self.close_at(q, b"script") {
            self.fill(q);
            self.set(SCRIPT_CLOSE);
            return Some(q);
        }
        self.embed(Emb::ScriptTag, 0, q, None)
    }

    fn script_other_body(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            let c = self.b[p];
            if c == b'<' && self.is(p, b"<!--") {
                self.tok(p, p + 4, COMMENT_BEGIN);
                p += 4;
                continue;
            }
            if (c == b'<' || c == b'-') && self.close_at(p, b"script") {
                self.fill(p);
                self.set(SCRIPT_CLOSE);
                return Some(p);
            }
            p += 1;
        }
        None
    }

    /// `script-close-tag`'s set: `-->`, then `</script` sets the closing tag's context.
    fn script_close(&mut self, mut p: usize) -> Option<usize> {
        while p < self.b.len() {
            if self.is(p, b"-->") {
                self.tok(p, p + 3, COMMENT_END);
                return Some(p + 3);
            }
            if self.is(p, b"</") && self.is_ic(p + 2, b"script") {
                self.tok(p, p + 2, TAG_BEGIN);
                self.tok(p + 2, p + 8, N_SCRIPT);
                self.set(TAG);
                return Some(p + 8);
            }
            p += 1;
        }
        None
    }

    // ---- embeds ----

    /// The first place from `from` where the embed's escape matches.
    fn escape(&self, emb: Emb, quote: u8, from: usize) -> Option<usize> {
        match emb {
            Emb::StyleTag => self.find_close(from, b"style"),
            Emb::ScriptTag => self.find_close(from, b"script"),
            Emb::StyleAttr | Emb::EventAttr => self.b[from.min(self.b.len())..]
                .iter()
                .position(|&c| c == quote)
                .map(|i| from + i),
        }
    }

    /// The embed `emb` from `from` (an attribute's from after its quote `quote`), its language's
    /// state `st` there (`None` for a fresh one): its spans to its escape, where HTML goes on
    /// (after an attribute's closing quote); `None` when the line ends inside it.
    fn embed(&mut self, emb: Emb, quote: u8, from: usize, st: Option<State>) -> Option<usize> {
        let n = self.b.len();
        let first = self.escape(emb, quote, from);
        self.fill(from);
        // a fresh `style="…"` is CSS's `rule-list-body`
        let st0 = st.unwrap_or_else(|| {
            if emb == Emb::StyleAttr {
                crate::css::rule_list_body()
            } else {
                State::default()
            }
        });
        let mut e = from;
        if first != Some(from) {
            let k = self.out.0.len();
            let window = if emb.attr() { WINDOW_ATTR } else { WINDOW };
            let mut end = first.map_or(n, |e| (e + window).min(n));
            loop {
                while end < n && self.b[end] & 0xc0 == 0x80 {
                    end += 1;
                }
                let mut st = st0;
                let b = &self.b[..end];
                self.out.1 = false;
                if emb.css() {
                    crate::css::line_from(b, from, &mut st, self.out);
                } else {
                    crate::javascript::line_from(b, from, &mut st, self.out);
                }
                self.out.1 = true;
                match self.mark(k, emb, quote, first, from, end, window) {
                    Mark::Escaped(at) => {
                        e = at;
                        break;
                    }
                    Mark::Inside => {
                        self.at = n;
                        self.emb = Some((emb, quote, st));
                        return None;
                    }
                    Mark::Wider(to) => {
                        // a token ran past the escape, and the next one is past what the scanner
                        // saw: again, seeing more of the line
                        self.out.0.truncate(k);
                        end = to;
                    }
                }
            }
        }
        self.at = e;
        if emb.attr() {
            let end = if quote == b'"' { ED_END } else { ES_END };
            self.tok(e, e + 1, end);
            return Some(e + 1);
        }
        Some(e)
    }

    /// The embedded spans from `k` on, the scanner having seen the line to `end`: set the embed's
    /// bit, and cut them at the escape (the
    /// first from `first`). A context's text is cut there; a token from before it is one match
    /// that runs past it (a `//` comment, an escape such as `\"`, `<<`), and the escape is the next
    /// one after it. The scanner joined no spans, so each is one token or one stretch of text.
    #[allow(clippy::too_many_arguments)]
    fn mark(
        &mut self,
        k: usize,
        emb: Emb,
        quote: u8,
        first: Option<usize>,
        from: usize,
        end: usize,
        window: usize,
    ) -> Mark {
        let n = self.b.len();
        let bit = if emb.css() { CSS_KIND } else { JS_KIND };
        let mut escape = first;
        let mut i = k;
        while i < self.out.0.len() {
            let s = self.out.0[i];
            if let Some(e) = escape {
                if s.start as usize >= e {
                    self.out.0.truncate(i);
                    return Mark::Escaped(e);
                }
                if s.end as usize > e {
                    let text = if emb.css() {
                        crate::css::is_text(s.kind)
                    } else {
                        crate::javascript::is_text(s.kind)
                    };
                    if text {
                        self.out.0[i] = Span {
                            end: e as u32,
                            kind: s.kind | bit,
                            ..s
                        };
                        self.out.0.truncate(i + 1);
                        return Mark::Escaped(e);
                    }
                    escape = self.escape(emb, quote, s.end as usize);
                    let seen = escape.map_or(n, |e| (e + window).min(n));
                    if end < n && seen > end {
                        return Mark::Wider(seen.max(end + (end - from)).min(n));
                    }
                }
            }
            self.out.0[i].kind |= bit;
            i += 1;
        }
        match escape {
            Some(e) => Mark::Escaped(e),
            None => Mark::Inside,
        }
    }
}

/// What an embed's spans come to ([`Scan::mark`]).
enum Mark {
    /// Its escape is at this place.
    Escaped(usize),
    /// The line ends inside it.
    Inside,
    /// Its scanner must see the line to this place.
    Wider(usize),
}
