//! pulldown-cmark events → [`Doc`] (ADR-01). A stack-based builder turns `Start`/`End` pairs into
//! owned blocks, resolves inline markup into styled runs, and converts byte offsets to lines.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::path::{Path, PathBuf};

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd,
};

use super::embed::{self, Embed, Embedded, Embeds, Offsets};
use super::flavor::Dialect;
use super::lines::{Scan, code_line_count};
use super::slug::Slugger;
use super::{
    Align, Anchor, AnchorKind, Block, BlockKind, Callout, CalloutKind, CodeBlock, CodeId, Doc,
    Fold, Footnote, FootnoteId, ImageSize, InlineStyle, Inlines, LineRange, LinkId, LinkKind,
    LinkRef, List, ListItem, MathId, Properties, Property, Run, RunKind, Slots, Table, TableId,
    TableRow, Task, TextRange, UnitId, fence, plain_text, props, tex,
};
use crate::guard;
use crate::links::Target;

/// The parser's options, for every dialect: what has no other meaning in CommonMark or GFM is on
/// everywhere, as wiki links and math are (ADR-45). Front matter is found by [`super::front`],
/// not by pulldown-cmark's option, which takes a `---` anywhere for it. Public for the throughput
/// bench's C2 ceiling, pulldown-cmark's events alone (BEN-019).
pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_MATH
}

/// Parse CommonMark into the IR: [`parse_as`] its dialect.
pub fn parse(text: &str) -> Doc {
    parse_as(text, Dialect::CommonMark)
}

/// Parse markdown of `dialect` into the IR. Never panics, whatever the input: when pulldown-cmark
/// panics, or makes events out of all proportion to the text, the text shows as plain (F-05).
pub fn parse_as(text: &str, dialect: Dialect) -> Doc {
    guard::catch(|| markdown(text, dialect, None))
        .flatten()
        .unwrap_or_else(|| super::file::code(text, None))
}

/// [`parse_as`] the note at `path`, transcluding the notes its embeds name from `embeds`, as a
/// load read them (OBS-007): an embed alone in its paragraph shows the note, or the heading's
/// section of it, in place.
pub fn parse_with(text: &str, dialect: Dialect, path: &Path, embeds: &Embeds) -> Doc {
    if embeds.is_empty() {
        return parse_as(text, dialect);
    }
    let budget = Budget::default();
    let stack = Stack {
        path,
        up: None,
        depth: 0,
    };
    let embedding = Embedding {
        embeds,
        stack: &stack,
        budget: &budget,
        dialect,
    };
    guard::catch(std::panic::AssertUnwindSafe(|| {
        markdown(text, dialect, Some(embedding))
    }))
    .flatten()
    .unwrap_or_else(|| super::file::code(text, None))
}

/// What a parse transcludes embeds from: the notes read, and the notes being parsed.
#[derive(Clone, Copy)]
struct Embedding<'a> {
    embeds: &'a Embeds,
    stack: &'a Stack<'a>,
    budget: &'a Budget,
    dialect: Dialect,
}

/// What one parse has spent on embeds, shared by the notes it transcludes.
#[derive(Default)]
struct Budget {
    /// The bytes of notes transcluded so far, against [`embed::MAX_TRANSCLUDED`].
    spent: Cell<usize>,
    /// The notes parsed whole to find an embed's section, each once a parse (`None`: it didn't
    /// parse): a note embedded by many of its sections is parsed for them once, so these parses
    /// are bounded by the notes a load reads (PAR-012).
    parsed: RefCell<Vec<(PathBuf, Option<Doc>)>>,
}

/// The notes being parsed, the innermost first: a note is never transcluded inside itself.
struct Stack<'a> {
    path: &'a Path,
    up: Option<&'a Stack<'a>>,
    depth: u32,
}

impl Stack<'_> {
    fn holds(&self, path: &Path) -> bool {
        self.path == path || self.up.is_some_and(|up| up.holds(path))
    }
}

/// The most events a text of `len` bytes may make. Notes make up to 0.22 a byte, and dense markup
/// (an empty table cell per `|`) about 2; before 0.13.5, pulldown-cmark doubled its events with
/// each `[[a|]]` (pulldown-cmark#1102). Kept as a net for the next such bug.
fn max_events(len: usize) -> usize {
    8 * len + 4096
}

/// [`parse`], or `None` once the text makes more than [`max_events`].
///
/// Inline math closes as pandoc's rule has it, past pulldown-cmark's (GitHub's): a `$` followed
/// by a digit doesn't close it, so prices (`$1.14 effective ($11.80 list`) stay text. Nor, past
/// pandoc, does one right after a backtick: that's how a code span that starts with a `$` opens
/// (`` `${@:-1}` ``, `` `$HOME` ``), and the code span takes it. Math that opens with a backtick
/// too is GitHub's other form (`` $`\sqrt{2}`$ ``) and stays math, as does a backtick elsewhere
/// in math, often a prime (`` $f`(x)$ ``). pulldown-cmark has no hook for its closers, so such
/// a `$` is masked (a byte the text lacks, of the same length, so every offset holds): before
/// the first pass where the text alone shows it opens no math ([`never_opens`]), as a price's
/// `$` does, so a note with prices parses once (PAR-010); else a text where pulldown-cmark closes
/// math on one is parsed again with it masked, until no such `$` is left or after
/// [`MAX_MATH_PASSES`]. Notes without one parse once, as before. In a note that leaves only a
/// control byte to mask with, a false closer the control byte would change the parse at is kept
/// instead: its math read as pulldown-cmark reads it ([`kept_besides`], PAR-015).
///
/// A pass ([`read_pass`]) is one loop, which inlines the parser's iterator and
/// [`Builder::event`] (PAR-010). Its builder stops at the first false closer, and the rest of
/// the pass only finds the others ([`Builder::dead`]), of which the next pass masks those
/// pandoc's rule skips as this one reads them: each paragraph's (heading's, cell's) first, and
/// the next only where masking it leaves the rest as it was ([`Builder::settle`], PAR-016). So
/// the masks are pandoc's, left to right, a label's copies with them ([`label_copies`],
/// [`stepwise`], PAR-019), up to [`MAX_MATH_PASSES`]. What masking costs is in [`set_bytes`] and
/// [`Builder::event_rare`], out of the way.
///
/// In Obsidian's dialect, its `%%comments%%` go first (OBS-004), their line ends kept: what's
/// parsed is the note without them, its lines numbered as the note's. A `%%` in code opens none,
/// the code as pulldown-cmark reads the note as written: a note whose `%%`s all look like code
/// is parsed as written, its first pass finding its code ([`passes`]), and again without its
/// comments only where that finds one; any other's code is found by a pass of its own first
/// ([`super::comments::code`], PAR-021). pulldown-cmark reads the
/// note from past its front matter (found by [`super::front`]; with a blank line after its `---`
/// in that dialect, OBS-011), its offsets moved back to the note's; the front matter is read into
/// the note's first block, its properties (ADR-46).
fn markdown(text: &str, dialect: Dialect, embedding: Option<Embedding<'_>>) -> Option<Doc> {
    let max = max_events(text.len());
    // the front matter as Obsidian reads it, taken below as the dialect does; an Excalidraw
    // drawing's makes the note Obsidian's, cut where the drawing's data starts (OBS-015)
    let read = super::front::read(text, true);
    let drawing = (read.as_ref())
        .filter(|f| f.excalidraw)
        .map(|f| super::front::drawing(text, f));
    let text = drawing.as_ref().map_or(text, |d| &text[..d.end]);
    let obsidian = matches!(dialect, Dialect::Obsidian { .. }) || drawing.is_some();
    // where the front matter ends, as the dialect takes it; comments start past it, which keeps
    // its place
    let matter = read.filter(|f| f.taken(obsidian)).map_or(0, |f| f.end);
    let front = drawing.as_ref().map_or(matter, |d| d.start);
    let note = Note {
        dialect,
        drawing: drawing.is_some(),
        obsidian,
        front,
        max,
        embedding,
    };
    if !obsidian || !super::comments::any(text) {
        return passes(text, first(text, front), &note, false)?.ok();
    }
    // a note whose `%%`s all look like code is parsed as written, its first pass finding its code
    // and so its comments (a drawing's text before `front` isn't parsed); any other's code is
    // found by a pass of its own (PAR-021)
    let looks = drawing.is_none() && super::comments::look_like_code(text, front);
    #[cfg(test)]
    let looks = looks && !tests::CODE_PASS.get();
    let comments = match looks {
        true => match passes(text, first(text, front), &note, true)? {
            Ok(doc) => return Some(doc),
            Err(comments) => comments,
        },
        false => {
            let code = super::comments::code(text, matter, max)?;
            let comments = super::comments::find(text, &code);
            if comments.is_empty() {
                return passes(text, first(text, front), &note, false)?.ok();
            }
            comments
        }
    };
    let uncommented = super::comments::strip(text, &comments)?;
    passes(&uncommented, first(&uncommented, front), &note, false)?.ok()
}

/// What [`markdown`] parses a note (or the note without its comments) with.
struct Note<'e> {
    dialect: Dialect,
    /// An Excalidraw drawing's: [`Builder::drawing`] reads its front matter and banner.
    drawing: bool,
    obsidian: bool,
    /// Where pulldown-cmark starts reading.
    front: usize,
    max: usize,
    embedding: Option<Embedding<'e>>,
}

/// The source of [`markdown`]'s first pass over `text`: `text` with the `$`s [`never_opens`]
/// finds masked ([`premasked`]), and the mask, or `text` as it is; and `text`'s lines, which
/// every pass reads.
struct First<'t> {
    source: Cow<'t, str>,
    mask: Option<u8>,
    /// The `$`s masked, when the mask is some.
    never: Vec<usize>,
    /// `text` holds a `$` past the front matter.
    dollars: bool,
    /// `text`'s line starts and marks ([`Builder::line_starts`], [`Builder::marks`]).
    starts: Vec<u32>,
    marks: Vec<u32>,
}

fn first(text: &str, front: usize) -> First<'_> {
    let mut source = Cow::Borrowed(text);
    let mut mask = None;
    // a note without a `$` past its front matter has no math, and no wiki link's target to read
    // for it; found in the newlines' pass (PAR-022)
    let Scan {
        starts,
        marks,
        dollar: dollars,
    } = super::lines::scan(text, front);
    let never = match dollars {
        true => never_opens(text, front),
        false => Vec::new(),
    };
    #[cfg(test)]
    let never = tests::hooked(never);
    let mut never = never;
    if !never.is_empty() {
        mask = premasked(&mut source, text, front, &mut never);
    }
    First {
        source,
        mask,
        never,
        dollars,
        starts,
        marks,
    }
}

/// [`markdown`]'s passes over `text`, the first from `start`: the doc, or `None` past
/// [`max_events`]. Where `comments` is set (an Obsidian note whose `%%`s all look like code), the
/// first pass finds `text`'s code too, and where that leaves a `%%` to open a comment, gives back
/// the comments ([`super::comments::find`]) instead of reading on: the code as pulldown-cmark
/// reads the note as written, so only from a first pass that masked no `$`; a masked one's is
/// found by a pass of [`super::comments::code`] (PAR-021).
fn passes(
    text: &str,
    start: First<'_>,
    note: &Note<'_>,
    comments: bool,
) -> Option<Result<Doc, Vec<Range<usize>>>> {
    let Note {
        dialect,
        drawing,
        obsidian,
        front,
        max,
        embedding,
    } = *note;
    let First {
        mut source,
        mut mask,
        never,
        dollars,
        mut starts,
        mut marks,
    } = start;
    // where the source's `$`s are masked, before the first pass and by the passes
    let mut masked_at = if mask.is_some() { never } else { Vec::new() };
    // the false closers left unmasked under a control byte, read as pulldown-cmark reads them
    // (PAR-015), in order
    let mut kept = Vec::new();
    // what the notes transcluded before this parse took: a pass after a false closer grafts its
    // embeds again, and counts them from here, not on top of the last pass's (PAR-012)
    let spent = embedding.map(|e| e.budget.spent.get());
    let mut pass = 0;
    // the pass that takes its math as pulldown-cmark reads it, false closers or not
    let mut last = false;
    loop {
        let mut b = Builder::with_lines(text, mem::take(&mut starts), mem::take(&mut marks));
        b.embedding = embedding;
        // a drawing's text elements keep their lines, as Obsidian's default does
        b.line_breaks = match dialect {
            Dialect::Obsidian { .. } => dialect.line_breaks(),
            _ => drawing,
        };
        b.obsidian = obsidian;
        b.dollars = dollars;
        if drawing {
            b.drawing(front);
        } else if front > 0 {
            b.properties();
        }
        b.mask = mask;
        #[cfg(test)]
        tests::MASK.set(mask);
        b.rare_from = masked_at.first().copied().unwrap_or(usize::MAX);
        b.masked = mem::take(&mut masked_at);
        b.kept = mem::take(&mut kept);
        b.last = last || pass == MAX_MATH_PASSES;
        // pandoc's rule takes as many passes as it takes
        #[cfg(test)]
        if tests::ideal() {
            b.last = last;
        }
        #[cfg(test)]
        tests::PASSES.set(tests::PASSES.get() + 1);
        if pass == 0 && comments {
            // the front matter, then what the pass finds
            b.code = Some(std::iter::once(0..front).collect());
            b.rare_from = 0;
        }
        read_pass(&mut b, &source, front, max)?;
        if let Some(mut code) = b.code.take() {
            let found = match mask {
                None => {
                    code.sort_unstable_by_key(|r| r.start);
                    super::comments::find(text, &code)
                }
                // the code as written, past masks that may have moved math (`$` prices)
                Some(_) => super::comments::find(text, &super::comments::code(text, front, max)?),
            };
            if !found.is_empty() {
                if let (Some(e), Some(spent)) = (embedding, spent) {
                    e.budget.spent.set(spent);
                }
                return Some(Err(found));
            }
        }
        // a builder that met a false closer stopped there ([`Builder::dead`])
        if !b.dead {
            return Some(Ok(b.finish()));
        }
        if let (Some(e), Some(spent)) = (embedding, spent) {
            e.budget.spent.set(spent);
        }
        // the last inline context's false closers
        b.settle(text.len());
        #[cfg(test)]
        tests::WRONG.with_borrow_mut(|w| w.extend_from_slice(&b.wrong));
        masked_at = mem::take(&mut b.masked);
        kept = mem::take(&mut b.kept);
        // `text`'s lines, for the next pass's builder
        starts = mem::take(&mut b.line_starts);
        marks = mem::take(&mut b.marks);
        pass += 1;
        // a text holding every candidate byte (only a fuzzer's): math as pulldown-cmark reads it
        let Some(m) = mask.or_else(|| mask_byte(text)) else {
            last = true;
            continue;
        };
        mask = Some(m);
        // those of each inline context that pandoc's rule skips, as this pass reads them (none
        // kept)
        let mut wrong = mem::take(&mut b.wrong);
        wrong.sort_unstable();
        wrong.dedup();
        let control = m.is_ascii_control();
        if control {
            // a control byte where a `$` would change the parse: the false closer is kept
            let all = wrong.clone();
            line_safe(text.as_bytes(), &mut wrong, true, false);
            kept_besides(&mut kept, &all, &wrong);
        }
        set_bytes(&mut source, &wrong, m);
        masked_at.extend_from_slice(&wrong);
        masked_at.sort_unstable();
        masked_at.dedup();
        // a label's `$` masked in each copy the math leaves, as the next pass reads it: after
        // each pass, as a later one may undo math a copy was in (PAR-014); a copy in math as
        // this pass's masks read it may be out of it as pandoc's rule meets them, one at a time
        // ([`stepwise`], PAR-019)
        let (mut copies, unmasked, rejected) =
            label_copies(&source, text, front, &masked_at, control);
        if !rejected.is_empty() && wrong.len() > 1 {
            copies.extend(stepwise(&source, front, &wrong, &copies, &rejected, m));
            copies.sort_unstable();
        }
        if !unmasked.is_empty() {
            set_bytes(&mut source, &unmasked, b'$');
            masked_at.retain(|at| unmasked.binary_search(at).is_err());
            kept.extend_from_slice(&unmasked);
            kept.sort_unstable();
        }
        while !copies.is_empty() {
            set_bytes(&mut source, &copies, m);
            masked_at.extend_from_slice(&copies);
            masked_at.sort_unstable();
            // a copy masked may undo math another copy was in: judged again, as the next pass
            // would, which may meet no false closer to call this after (PAR-019)
            copies = label_copies(&source, text, front, &masked_at, control).0;
        }
        #[cfg(test)]
        tests::KEPT.set(kept.len());
    }
}

/// One pass of [`markdown`]: the events of `source` past `front` read into `b`, their offsets
/// moved by `front`, or `None` past [`max_events`]' `max`. The loop is v1.1.0's, but for how it
/// counts the events: it calls the parser's iterator and [`Builder::event`] once each, and is
/// their one caller, so both are inlined into it. Nothing in it is the same for a whole pass: a test of the mask was, LLVM
/// unswitched the loop on it into two loops, each a caller, and both went out of line, every
/// note's parse +8 to +17% instructions (PAR-010). The mask is the builder's
/// ([`Builder::mask`]), tested where its opening is, and the false closers are its too
/// ([`Builder::wrong`]), found in its arm for inline math. It is also the one reader of
/// pulldown-cmark's events in ada: the parses that only look at them read them here too, into a
/// builder that probes ([`probe`]). A second reader, of a parser type of its own, was a second
/// caller of pulldown-cmark's `ItemBody::clone`, which went out of line, a call an event
/// (PAR-023).
#[inline(never)]
fn read_pass(b: &mut Builder<'_>, source: &str, front: usize, max: usize) -> Option<()> {
    let events = Parser::new_ext(&source[front..], options()).into_offset_iter();
    // counted down, not `enumerate`d: its tuple moved each event once more, read_pass +1.8 M
    // instructions on XL (PAR-023)
    let mut left = max;
    for (event, range) in events {
        if left == 0 {
            return None;
        }
        left -= 1;
        b.event(event, range.start + front..range.end + front);
    }
    Some(())
}

/// What a parse that only looks at pulldown-cmark's events finds ([`probe`]).
pub(super) enum Probe {
    /// The code a `%%` opens no comment in ([`super::comments::is_code`]), for
    /// [`super::comments::code`].
    Code(Vec<Range<usize>>),
    /// The ranges of math and wiki links, for [`math_ranges`].
    Math(Vec<Range<usize>>),
    /// Each inline math's range and its false closer ([`wrong_closer`]), for
    /// [`Builder::wiki_target`].
    Wiki(Vec<(Range<usize>, usize)>),
}

impl Probe {
    /// `event`, at `range` of `source`, looked at.
    fn event(&mut self, source: &str, event: &Event<'_>, range: Range<usize>) {
        match self {
            Probe::Code(code) => {
                if super::comments::is_code(event) {
                    code.push(range);
                }
            }
            Probe::Math(math) => {
                let found = matches!(
                    event,
                    Event::InlineMath(_)
                        | Event::DisplayMath(_)
                        | Event::Start(
                            Tag::Link {
                                link_type: LinkType::WikiLink { .. },
                                ..
                            } | Tag::Image {
                                link_type: LinkType::WikiLink { .. },
                                ..
                            }
                        )
                );
                if found {
                    math.push(range);
                }
            }
            Probe::Wiki(wrong) => {
                if let Event::InlineMath(t) = event
                    && let Some(at) = wrong_closer(source, &range, t.len())
                {
                    wrong.push((range, at));
                }
            }
        }
    }
}

/// `what` with the events of `source` past `front` looked at, their offsets moved by `front`;
/// and whether the parse ended within [`max_events`]' `max` (past it, `what` holds the first
/// `max` events'). Through [`read_pass`], pulldown-cmark's one reader (PAR-023), into a builder
/// that builds nothing ([`Builder::probing`]).
#[cold]
#[inline(never)]
pub(super) fn probe(source: &str, front: usize, max: usize, what: Probe) -> (Probe, bool) {
    let mut b = Builder::probing(source, what);
    let done = read_pass(&mut b, source, front, max).is_some();
    (
        b.probe.take().expect("a probing builder keeps its probe"),
        done,
    )
}

/// `source` with the bytes at `at` set to `byte`, for the next pass of [`markdown`]: a `$` to
/// the mask, or a masked `$` back. Both are ASCII.
#[cold]
#[inline(never)]
fn set_bytes(source: &mut Cow<'_, str>, at: &[usize], byte: u8) {
    if at.is_empty() {
        return;
    }
    let mut bytes = mem::take(source).into_owned().into_bytes();
    for &i in at {
        bytes[i] = byte;
    }
    *source = Cow::Owned(String::from_utf8(bytes).expect("`$` and the mask are both ASCII"));
}

/// The byte [`markdown`] masks `text`'s `$`s with ([`unused_byte`]), or a control byte in the
/// tests that force one.
fn mask_byte(text: &str) -> Option<u8> {
    let m = unused_byte(text)?;
    #[cfg(test)]
    let m = tests::forced(text).unwrap_or(m);
    Some(m)
}

/// `kept`, in order, with the false closers of `all` that [`line_safe`] left out of `safe`
/// (both in order): under a control byte, the passes of [`markdown`] leave those unmasked and
/// read their math as pulldown-cmark does, as the last pass would (PAR-015). A control byte is
/// no punctuation, so one next to a delimiter run may make it flank otherwise (`x~$3 … ~$3`:
/// a strikethrough comes or goes), and one in a link's destination or an autolink breaks it.
#[cold]
fn kept_besides(kept: &mut Vec<usize>, all: &[usize], safe: &[usize]) {
    if all.len() > safe.len() {
        kept.extend(all.iter().filter(|at| safe.binary_search(at).is_err()));
        kept.sort_unstable();
    }
}

/// The `$`s to mask beside `masked`, those masked so far in order (the false closers the passes of
/// [`markdown`] met, and their copies), in `source` (the next pass's, of `text` past `front`), and
/// those of `masked` to give back: where one is in a label (between brackets on its line: a
/// footnote's, a link reference's), the same `$` of each copy of that label (PAR-014).
/// pulldown-cmark matches a reference to its definition by the label's text, so a `$` masked in one
/// copy but not in the other unmatched them (`$1 [^a$1]` … `[^a$1]: b`). A copy is a bracketed
/// group on one line whose text is the label's but for case and runs of blanks, as pulldown-cmark
/// compares them, and its `$` is the label's same one, counted in `$`s. Not a copy whose `$` is
/// math's in `source` ([`math_ranges`]), which makes it no reference, as the `$` opening it would
/// no longer: returned apart, each with the masked `$` that made its label's one to mask, for
/// [`stepwise`]. Called after each pass, and again while it masks copies (a masked copy may undo
/// math another was in), a copy whose math a later mask undoes is masked then (PAR-019). So a
/// label's `$` is masked in every copy the math leaves, definitions above all (block-level, never
/// math's), and masking one the math leaves changes no math (it can close, as the false closer
/// could, so math reaching it would have closed there). A copy that is a link's text is given back
/// with its text, one in a definition's destination or title at the links that use it
/// ([`Builder::add_masked_link`]). Under a `control` byte, a label's `$` that a control byte would
/// change the parse at in any copy ([`line_safe`]: next to a delimiter run, in a destination; in
/// the rare note using every punctuation byte) is masked in none (PAR-015), and given back where it
/// is: the false closer is kept ([`kept_besides`]), so its math is read as pulldown-cmark reads it,
/// and every copy of the label stays the same. That's decided by the copies in the text, math or
/// not, so it holds for every pass. A wiki link's target is no label. Only a label with a
/// definition ([`defines`]): one nothing defines makes no reference, its copies text however
/// they're masked. Linear in the text past the probe's parse, made only when a `$` masked is in
/// such a label.
#[cold]
#[inline(never)]
fn label_copies(
    source: &str,
    text: &str,
    front: usize,
    masked: &[usize],
    control: bool,
) -> (Vec<usize>, Vec<usize>, Vec<(usize, usize)>) {
    let b = text.as_bytes();
    // each label's text, as compared: the `$`s of it to mask, counted from its first, and
    // whether it has a definition
    // each with the first masked `$` of it, the copy that made it a label's to mask
    let mut labels: HashMap<String, (Vec<(usize, usize)>, bool)> = HashMap::new();
    for &at in masked {
        if let Some((open, close)) = label_at(b, at) {
            let nth = memchr::memchr_iter(b'$', &b[open + 1..at]).count();
            let (nths, _) = labels.entry(label_key(&text[open + 1..close])).or_default();
            if !nths.iter().any(|&(n, _)| n == nth) {
                nths.push((nth, at));
            }
        }
    }
    let (mut copies, mut unmasked, mut rejected) = (Vec::new(), Vec::new(), Vec::new());
    if labels.is_empty() {
        return (copies, unmasked, rejected);
    }
    // the copies' `$`s, in order, each with its label's text and which of its `$`s it is, and
    // the masked `$` that made it one to mask
    let mut found: Vec<(usize, String, usize, usize)> = Vec::new();
    for open in memchr::memchr_iter(b'[', &b[front..]).map(|i| front + i) {
        let Some(close) = memchr::memchr3(b'[', b']', b'\n', &b[open + 1..]).map(|i| open + 1 + i)
        else {
            break;
        };
        let inner = &b[open + 1..close];
        if b[close] != b']' || memchr::memchr(b'$', inner).is_none() || !is_label(b, open, close) {
            continue;
        }
        let key = label_key(&text[open + 1..close]);
        let Some((nths, defined)) = labels.get_mut(&key) else {
            continue;
        };
        *defined |= defines(b, open, close);
        let dollars: Vec<usize> = memchr::memchr_iter(b'$', inner).collect();
        for &(n, by) in nths.iter() {
            if let Some(d) = dollars.get(n) {
                found.push((open + 1 + d, key.clone(), n, by));
            }
        }
    }
    // a label nothing defines makes no reference: its copies are text however they're masked
    found.retain(|(_, key, ..)| labels[key].1);
    if control {
        let mut all: Vec<usize> = found.iter().map(|f| f.0).collect();
        all.sort_unstable();
        all.dedup();
        let mut safe = all.clone();
        line_safe(b, &mut safe, true, true);
        if safe.len() < all.len() {
            // each label's `$` a copy can't take a control byte at
            let unsafe_: Vec<(String, usize)> = (found.iter())
                .filter(|f| safe.binary_search(&f.0).is_err())
                .map(|f| (f.1.clone(), f.2))
                .collect();
            let mut kept: Vec<usize> = Vec::new();
            found.retain(|(at, key, n, _)| {
                let keep = !unsafe_.iter().any(|(k, m)| k == key && m == n);
                if !keep && masked.binary_search(at).is_ok() {
                    kept.push(*at);
                }
                keep
            });
            kept.sort_unstable();
            kept.dedup();
            unmasked = kept;
        }
    }
    found.retain(|(at, ..)| masked.binary_search(at).is_err());
    if found.is_empty() {
        return (copies, unmasked, rejected);
    }
    let math = math_ranges(source, front);
    for (at, _, _, by) in found {
        match in_math(&math, at) {
            false => copies.push(at),
            true => rejected.push((at, by)),
        }
    }
    copies.sort_unstable();
    copies.dedup();
    rejected.sort_unstable();
    rejected.dedup_by_key(|r| r.0);
    (copies, unmasked, rejected)
}

/// Whether `at` is in one of `math`'s ranges ([`math_ranges`]: in order, apart).
fn in_math(math: &[Range<usize>], at: usize) -> bool {
    let i = math.partition_point(|r| r.end <= at);
    math.get(i).is_some_and(|r| r.start <= at)
}

/// The copies of `rejected` ([`label_copies`]': each a copy in math as the next pass reads
/// `source`, with the masked `$` that made its label's one to mask) that are out of math as
/// `wrong` (the false closers this pass masks, in order) masked one at a time read them, from
/// the one that made each one to mask: pandoc's rule meets them so, and masks such a copy
/// (PAR-019). A pass masks the first false closer of each inline context at once
/// ([`Builder::settle`]), and a copy in one context may be in math only once another context's
/// mask moves that context's reading (a copy inside `$$…$$` that a lone `$` past it kept from
/// closing). `copies`, masked already, are masked at each step. A probe parse for each false
/// closer of `wrong` but its last, while a copy is left; only for a pass that masked several and
/// left a copy in math.
#[cold]
#[inline(never)]
fn stepwise(
    source: &str,
    front: usize,
    wrong: &[usize],
    copies: &[usize],
    rejected: &[(usize, usize)],
    m: u8,
) -> Vec<usize> {
    let mut bytes = source.as_bytes().to_vec();
    for &at in &wrong[1..] {
        bytes[at] = b'$';
    }
    for &at in copies {
        bytes[at] = m;
    }
    let mut left = rejected.to_vec();
    let mut out = Vec::new();
    for i in 1..wrong.len() {
        let Ok(step) = std::str::from_utf8(&bytes) else {
            break;
        };
        let math = math_ranges(step, front);
        // as masked so far: those of `wrong` before the `i`th, and those before this pass
        let masked = |at: usize| wrong[i..].binary_search(&at).is_err();
        let mut now = Vec::new();
        left.retain(|&(at, by)| {
            let out_of_math = masked(by) && !in_math(&math, at);
            if out_of_math {
                now.push(at);
            }
            !out_of_math
        });
        for &at in &now {
            bytes[at] = m;
        }
        out.extend(now);
        if left.is_empty() {
            break;
        }
        bytes[wrong[i]] = m;
    }
    out
}

/// Whether the label in the brackets at `open` and `close` may be a definition's
/// ([`label_copies`]): a `:` after it, and before it on its line only blanks and what opens a
/// quote or a list item. Generous: a guess that errs only toward a probe that finds nothing to
/// mask.
fn defines(b: &[u8], open: usize, close: usize) -> bool {
    let start = memchr::memrchr(b'\n', &b[..open]).map_or(0, |i| i + 1);
    b.get(close + 1) == Some(&b':')
        && (b[start..open].iter()).all(|c| b" \t>-*+.)0123456789".contains(c))
}

/// The ranges of `source` past `front` that are math (inline or display) as pulldown-cmark reads
/// it, or a wiki link, whose target's math makes no event ([`Builder::wiki_target`]) and holds
/// no reference, for [`label_copies`]: a parse of its own, events only ([`probe`]), of the first
/// [`max_events`]. In order, overlaps merged.
#[cold]
fn math_ranges(source: &str, front: usize) -> Vec<Range<usize>> {
    let max = max_events(source.len());
    let (Probe::Math(mut ranges), _) = probe(source, front, max, Probe::Math(Vec::new())) else {
        unreachable!("a probe gives back its own kind");
    };
    ranges.sort_unstable_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The brackets around the `$` at `at` on its line, where they may be a label's ([`is_label`]).
fn label_at(b: &[u8], at: usize) -> Option<(usize, usize)> {
    let open = memchr::memrchr3(b'[', b']', b'\n', &b[..at]).filter(|&i| b[i] == b'[')?;
    let close = memchr::memchr3(b'[', b']', b'\n', &b[at + 1..])
        .map(|i| at + 1 + i)
        .filter(|&i| b[i] == b']')?;
    is_label(b, open, close).then_some((open, close))
}

/// Whether the brackets at `open` and `close` may hold a label ([`label_copies`]): not a wiki
/// link's target. A link's text before a `(` may be (`[a](` with no destination is a reference).
fn is_label(b: &[u8], open: usize, close: usize) -> bool {
    !(open > 0 && b[open - 1] == b'[' && b.get(close + 1) == Some(&b']'))
}

/// A label's text as pulldown-cmark compares labels: case folded, runs of blanks one space.
fn label_key(label: &str) -> String {
    let mut key = String::with_capacity(label.len());
    for word in label.split_ascii_whitespace() {
        if !key.is_empty() {
            key.push(' ');
        }
        key.push_str(word);
    }
    key.to_lowercase()
}

/// Where `$`s pulldown-cmark may close inline math on and pandoc wouldn't are in `text` past
/// `from`, those the text alone shows open no math: masked before [`markdown`]'s first pass
/// ([`premasked`]), a note with prices parses once, where each `$` it closed math on cost it a
/// pass more (PAR-010; the vault's parse 4.6%).
///
/// Only a `$` math can reach is masked: the `$` before it in its paragraph can open (no blank
/// after it), or is a false closer math reached in turn. Any other `$` before it closes that
/// math or ends it, and one no math reaches is text either way: masking it would only cost its
/// note a copy and send events the slow way ([`Builder::rare_from`]). That's a guess, which may
/// err the other way: with no math open, a `$` after an odd number of backtick runs on its line
/// is taken to be in a code span, which takes it first, and to neither open math nor end it
/// (`` `$VISUAL` / `$EDITOR` ``, `` `-Xmx$HEAP` ``), and no math to reach across a `|`, as in
/// a table, whose cells are blocks of their own; where it does, the passes find the false
/// closer. Math open before a code span does reach its `$`s: pulldown-cmark pairs `$`s
/// before it reads the span.
///
/// Masking a `$` changes the parse only where it would act. pulldown-cmark pairs an inline
/// opener with the next `$` of its block and brace context: if that one can close (no blank
/// before it), it does, else the opener is text. So a `$` that can close and is followed by a
/// digit either closes math, falsely, and is masked by a later pass anyway; or opens math, which
/// masking would lose; or is text either way. It opens none when the next unescaped `$` in its
/// paragraph that isn't another price's has a blank before it, or there's none
/// ([`next_opens_none`]). Nor is one after a `$$` in its paragraph masked: a lone `$` keeps a
/// display span from closing over it. A `$` right after a backtick closes math falsely too,
/// unless the math opened on one, so one is masked the same way where no `` $` `` is before it in
/// its paragraph (and no backtick after it: `` `$` ``): that's how a code span starting on a `$`
/// opens, `` `${@:-1}` ``.
/// One between brackets on its line is a label's (a footnote's, a link reference's), matched by
/// its text, so its `$` is masked in every copy of the label or in none ([`label_masks`], each
/// copy read by these rules, math reaching it or not), and the note parses once where the passes
/// met such a false closer a pass (PAR-017). Not one with a bracket on one side only, nor in a
/// wiki link's target, which the rule below leaves to the passes. Nor one after a wiki link
/// holding a `$` in its paragraph ([`dollar_wiki_links`]): math opening in it may take its `]]`, and the link run on
/// to a later `]]` (`![[c$1]] … a$2 … $4]]`), or a later mask free one that math took and the
/// link take what came before (`![[a$2 … $4 … [[ … c$1]] …⏎ ~$3`, the `$3` closing the math over
/// the `]]`). A wiki link reads back over math that closed in it, so a `$` masked early there
/// changes the parse before it. The rest are left to the passes. Empty for a note without a
/// `$`, one `memchr`; linear in the text, whatever the prices.
fn never_opens(text: &str, from: usize) -> Vec<usize> {
    let b = text.as_bytes();
    let mut never = Vec::new();
    // the last `$$`, the last `$` opening math GitHub's way (`` $`…`$ ``), and the last `$`
    // while math opened before can close on the next one
    let (mut display, mut tick) = (Since::default(), Since::default());
    let mut reach = Since::default();
    // whether an odd number of backtick runs is on the line before the `$`, and how far that's
    // read (to the last `$`)
    let (mut in_code, mut ticks_read) = (false, from);
    // the last bracket on the line before the `$` (0 for none) and how far that's read; the first
    // after it (0 for none, at the line's end), known for any `$` before where it was found
    let (mut before, mut read) = (0, from);
    let mut after = (from, 0);
    // where the last [`next_opens_none`] stopped, and what it found: a price's `$` it went past
    // finds the same, so a paragraph of prices is read once
    let mut scan = (from, false);
    let mut braces = None;
    // the wiki links holding a `$` ([`dollar_wiki_links`]), found at the first `$` to mask; the
    // next of them past the last `$` read, and whether a blank line is known between the last
    // one before it and it, and how far that's read
    let mut wikis: Option<Vec<usize>> = None;
    let (mut wiki, mut wiki_blank, mut wiki_read) = (0, true, from);
    // the false closers between brackets on their lines ([`label_masks`])
    let mut labeled: Vec<(usize, bool, bool)> = Vec::new();
    for c in memchr::memchr_iter(b'$', &b[from..]).map(|i| i + from) {
        let next = b.get(c + 1).copied();
        if next == Some(b'`') {
            tick.set(c);
        }
        if escaped(b, c) {
            continue;
        }
        let can_close = c > from && !blank_byte(b[c - 1]);
        if can_close && b[c - 1] == b'$' {
            display.set(c);
        }
        let price = next.is_some_and(|n| n.is_ascii_digit());
        let after_tick = can_close && b[c - 1] == b'`' && next != Some(b'`');
        let false_closer = can_close && b[c - 1] != b'$' && (price || after_tick);
        // math that can close on `c` is open: the `$` before it in its paragraph can open, or
        // is a false closer math reached, which the passes mask; any other `$` closes the math
        // or ends it. With no math open, one in a code span is the span's (`` `$HOME` ``)
        // nor across a `|`, a table's cells being blocks of their own
        let reached = reach.holds(b, c) && memchr::memchr(b'|', &b[ticks_read..c]).is_none();
        let line =
            memchr::memrchr(b'\n', &b[ticks_read..c]).map_or(ticks_read, |i| ticks_read + i + 1);
        if line > ticks_read {
            in_code = false;
        }
        in_code ^= tick_runs(&b[line..c]) % 2 == 1;
        ticks_read = c;
        let unread = in_code && !reached;
        if !unread {
            let opens = next.is_some_and(|n| !blank_byte(n));
            if opens || (false_closer && reached) {
                reach.set(c);
            } else {
                reach.clear();
            }
        }
        if !false_closer {
            continue;
        }
        if let Some(i) = memchr::memrchr3(b'[', b']', b'\n', &b[read..c]) {
            before = b[read + i];
        }
        read = c;
        if c >= after.0 {
            after = match memchr::memchr3(b'[', b']', b'\n', &b[c + 1..]) {
                Some(i) => (c + 1 + i, b[c + 1 + i]),
                None => (b.len(), 0),
            };
        }
        // between brackets on its line: a label's copies are masked all or none, after the
        // loop, reached or not (PAR-017); a bracket on one side only, left to the passes
        let label = before == b'[' && after.1 == b']';
        if !label && (before == b'[' || after.1 == b']' || !reached) {
            continue;
        }
        if c >= scan.0 {
            scan = next_opens_none(b, c, &mut braces);
        }
        let mut ok = scan.1 && !display.holds(b, c) && (price || !tick.holds(b, c));
        if ok {
            let wikis = wikis.get_or_insert_with(|| dollar_wiki_links(b, from));
            while wikis.get(wiki).is_some_and(|&w| w < c) {
                (wiki_blank, wiki_read) = (false, wikis[wiki]);
                wiki += 1;
            }
            if !wiki_blank {
                wiki_blank = memchr::memchr_iter(b'\n', &b[wiki_read..c])
                    .any(|i| blank_line(b, wiki_read + i + 1));
                wiki_read = c;
                ok = wiki_blank;
            }
        }
        match label {
            true => labeled.push((c, ok, reached)),
            false if ok => never.push(c),
            false => {}
        }
    }
    if !labeled.is_empty() {
        label_masks(text, from, &labeled, &mut never);
    }
    never
}

/// The `$`s of `labeled` [`never_opens`] masks, into `never` (kept in order): the false closers
/// between brackets on their lines, in order, each with whether masking it changes nothing but
/// the passes (the rules of [`never_opens`], math reaching it or not), and whether math may
/// reach it. pulldown-cmark matches a reference to its definition by the label's text, so a
/// label's `$` is masked in every copy of the label or in none ([`label_copies`]' copies): in
/// all where each may be masked and math may reach one, which is the false closer the passes
/// would meet; else in none, left to the passes. A label nothing defines makes no reference
/// ([`defines`]): its `$`s are masked as any other's. Not a wiki link's target ([`is_label`];
/// the brackets in one are after a wiki link holding a `$`, which [`never_opens`] leaves to the
/// passes). Linear in the text (PAR-017).
#[cold]
#[inline(never)]
fn label_masks(text: &str, from: usize, labeled: &[(usize, bool, bool)], never: &mut Vec<usize>) {
    let labels = Labels::new(text, from);
    // as [`never_opens`] read it: may be masked, and reached
    let read = |at: usize| {
        let i = labeled.partition_point(|l| l.0 < at);
        labeled.get(i).filter(|l| l.0 == at).map(|l| (l.1, l.2))
    };
    // each label's `$` to mask, once: the label and which of its `$`s
    let mut wanted: Vec<(usize, usize)> = (labeled.iter())
        .filter(|l| l.1 && l.2)
        .filter_map(|l| labels.find(l.0))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let start = never.len();
    for (key, nth) in wanted {
        let copies = labels.copies(key, nth);
        if !labels.defined[key] {
            never.extend(copies.filter(|&at| read(at) == Some((true, true))));
        } else if copies.clone().all(|at| read(at).is_some_and(|(ok, _)| ok)) {
            never.extend(copies);
        }
    }
    if never.len() > start {
        never.sort_unstable();
        never.dedup();
    }
}

/// `never` (in order) less each defined label's `$` of which [`line_safe`] left out a copy that
/// [`label_masks`] masked: masked all or none (PAR-017).
#[cold]
fn untie(text: &str, from: usize, never: &mut Vec<usize>) {
    let labels = Labels::new(text, from);
    let mut tied: Vec<(usize, usize)> = (never.iter())
        .filter_map(|&at| labels.find(at))
        .filter(|&(key, _)| labels.defined[key])
        .collect();
    tied.sort_unstable();
    tied.dedup();
    let mut out: Vec<usize> = Vec::new();
    for (key, nth) in tied {
        let mut copies = labels.copies(key, nth);
        if !copies.clone().all(|at| never.binary_search(&at).is_ok()) {
            out.extend(copies.by_ref());
        }
    }
    if !out.is_empty() {
        out.sort_unstable();
        never.retain(|at| out.binary_search(at).is_err());
    }
}

/// The labels of a text past a point that hold a `$`, as [`label_copies`] finds their copies:
/// bracketed groups on one line, not a wiki link's target ([`is_label`]), each group's label its
/// text as compared ([`label_key`]). For [`label_masks`] and [`untie`]; linear in the text.
struct Labels {
    /// Each group's brackets, its label and its `$`s, in order.
    groups: Vec<(usize, usize, usize, Vec<usize>)>,
    /// Each label's groups.
    members: Vec<Vec<usize>>,
    /// Whether a group of each label may define it ([`defines`]).
    defined: Vec<bool>,
}

impl Labels {
    fn new(text: &str, from: usize) -> Self {
        let b = text.as_bytes();
        let mut keys: HashMap<String, usize> = HashMap::new();
        let (mut groups, mut members, mut defined) = (Vec::new(), Vec::new(), Vec::new());
        for open in memchr::memchr_iter(b'[', &b[from..]).map(|i| from + i) {
            let Some(close) =
                memchr::memchr3(b'[', b']', b'\n', &b[open + 1..]).map(|i| open + 1 + i)
            else {
                break;
            };
            let inner = &b[open + 1..close];
            if b[close] != b']'
                || memchr::memchr(b'$', inner).is_none()
                || !is_label(b, open, close)
            {
                continue;
            }
            let n = keys.len();
            let key = *keys.entry(label_key(&text[open + 1..close])).or_insert(n);
            if key == n {
                members.push(Vec::new());
                defined.push(false);
            }
            members[key].push(groups.len());
            defined[key] |= defines(b, open, close);
            let dollars = memchr::memchr_iter(b'$', inner)
                .map(|d| open + 1 + d)
                .collect();
            groups.push((open, close, key, dollars));
        }
        Self {
            groups,
            members,
            defined,
        }
    }

    /// The label of the `$` at `at` and which of its `$`s it is, if it's in one.
    fn find(&self, at: usize) -> Option<(usize, usize)> {
        let i = self.groups.partition_point(|g| g.1 < at);
        let (_, _, key, dollars) = self.groups.get(i).filter(|g| g.0 < at)?;
        dollars.binary_search(&at).ok().map(|nth| (*key, nth))
    }

    /// Where the `nth` `$` of each copy of the label `key` is, in order.
    fn copies(&self, key: usize, nth: usize) -> impl Iterator<Item = usize> + Clone + '_ {
        (self.members[key].iter()).filter_map(move |&g| self.groups[g].3.get(nth).copied())
    }
}

/// Where each wiki link (`[[`) of `b` past `from` holding a `$` before its `]]` or its line's end
/// starts, in order ([`never_opens`]).
#[cold]
fn dollar_wiki_links(b: &[u8], from: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = from;
    while let Some(open) = memchr::memmem::find(&b[at..], b"[[").map(|i| at + i) {
        let end = memchr::memchr(b'\n', &b[open..]).map_or(b.len(), |i| open + i);
        let close = memchr::memmem::find(&b[open + 2..end], b"]]").map_or(end, |i| open + 2 + i);
        if memchr::memchr(b'$', &b[open + 2..close]).is_some() {
            out.push(open);
        }
        at = close;
    }
    out
}

/// The last `$` of a kind [`never_opens`] has read, while no blank line is known past it.
#[derive(Default)]
struct Since {
    at: Option<usize>,
    /// How far no blank line is known past it.
    seen: usize,
}

impl Since {
    fn clear(&mut self) {
        self.at = None;
    }

    fn set(&mut self, at: usize) {
        *self = Self {
            at: Some(at),
            seen: at,
        };
    }

    /// Whether the `$` is in the paragraph of the one at `c`, before it.
    fn holds(&mut self, b: &[u8], c: usize) -> bool {
        if self.at.is_some() {
            if (memchr::memchr_iter(b'\n', &b[self.seen..c]))
                .any(|n| blank_line(b, self.seen + n + 1))
            {
                self.at = None;
            }
            self.seen = c;
        }
        self.at.is_some()
    }
}

/// The runs of backticks in `b`, a run cut at its start.
fn tick_runs(b: &[u8]) -> usize {
    memchr::memchr_iter(b'`', b)
        .filter(|&i| i == 0 || b[i - 1] != b'`')
        .count()
}

/// pulldown-cmark's blank: an ASCII space, tab, line feed, vertical tab, form feed or return.
fn blank_byte(c: u8) -> bool {
    matches!(c, b'\t'..=b'\r' | b' ')
}

/// Whether the byte at `at` is escaped: an odd run of backslashes before it.
fn escaped(b: &[u8], at: usize) -> bool {
    b[..at].iter().rev().take_while(|&&c| c == b'\\').count() % 2 == 1
}

/// Whether `c` opens no math ([`never_opens`]), and where that was found: the next unescaped `$`
/// after it in its paragraph that isn't another price's has a blank before it, or there's none.
/// Math `c` opened would close on a price's `$` first, falsely, and the passes mask it, so `c`'s
/// math is closed by the next `$` past those, if any; a backtick's `$` closes it falsely too,
/// as `c` is no `` $` ``. A price past it may open math once `c`'s fails, as pandoc's rule and
/// the passes read it (PAR-016), but would close where `c`'s would, and so on none: not a
/// `` `$` ``, which opens GitHub's. A paragraph ends at a blank line, or sooner: a `$` past its
/// end closes nothing of `c`'s either way.
///
/// Braces between are balanced braces with no `$` in them, or `c` is left to the passes: a `$`
/// inside braces is in another context, and a `}` past `c`'s moves its context. Balanced, they
/// leave the `$`s past them as they were, but for their contexts' numbers: with the first `$` of
/// a block masked its braces aren't counted, and the numbers past them change. Those are a byte
/// and the root's is 255, less one for each unbalanced `}`, so they could meet only past 255
/// braces in all, `braces` (counted once a note, at its first brace).
fn next_opens_none(b: &[u8], c: usize, braces: &mut Option<bool>) -> (usize, bool) {
    let mut depth = 0u32;
    let mut i = c + 1;
    // a `$` or a line's end at a time, the bytes between looked at only for a brace; past
    // `MAX_SCAN` without an answer, `c` is left to the passes (a paragraph is read here to a
    // blank line, through code blocks, lists and tables)
    loop {
        if i - c > MAX_SCAN {
            return (i, false);
        }
        let j = memchr::memchr2(b'$', b'\n', &b[i..]).map_or(b.len(), |k| i + k);
        if memchr::memchr2(b'{', b'}', &b[i..j]).is_some() {
            if !*braces.get_or_insert_with(|| few_braces(b)) {
                return (i, false);
            }
            for &x in &b[i..j] {
                match x {
                    b'{' => depth += 1,
                    b'}' if depth == 0 => return (i, false),
                    b'}' => depth -= 1,
                    _ => {}
                }
            }
        }
        if j == b.len() {
            return (j, depth == 0);
        }
        match b[j] {
            b'$' if depth > 0 => return (j, false),
            b'$' if !escaped(b, j) => {
                // closes `c`'s math falsely, before a digit or after a backtick (`c`'s math opens
                // on none), and opens what `c` would: not a `` `$` ``, which opens GitHub's
                let next = b.get(j + 1).copied();
                let price = !blank_byte(b[j - 1])
                    && b[j - 1] != b'$'
                    && (next.is_some_and(|n| n.is_ascii_digit())
                        || (b[j - 1] == b'`' && next != Some(b'`')));
                if !price {
                    return (j, blank_byte(b[j - 1]));
                }
            }
            b'\n' if blank_line(b, j + 1) => return (j, depth == 0),
            _ => {}
        }
        i = j + 1;
    }
}

/// How far [`next_opens_none`] reads past a `$` for its answer.
const MAX_SCAN: usize = 4096;

/// Whether `b` holds too few braces for their contexts' numbers to meet ([`next_opens_none`]).
#[cold]
fn few_braces(b: &[u8]) -> bool {
    memchr::memchr2_iter(b'{', b'}', b).take(255).count() < 255
}

/// Whether the line at `start` is blank, or the text ends there.
fn blank_line(b: &[u8], start: usize) -> bool {
    (b[start..]
        .iter()
        .find(|&&c| !matches!(c, b' ' | b'\t' | b'\r')))
    .is_none_or(|&c| c == b'\n')
}

/// `source` with the `$`s at `never` masked before [`markdown`]'s first pass, and the mask, or
/// `None`, `source` left as it is; `never` less those [`line_safe`] leaves to the passes, and
/// less the other copies of a label one of them is in ([`untie`]).
#[cold]
#[inline(never)]
fn premasked(
    source: &mut Cow<'_, str>,
    text: &str,
    from: usize,
    never: &mut Vec<usize>,
) -> Option<u8> {
    let m = mask_byte(text)?;
    let all = never.len();
    line_safe(text.as_bytes(), never, m.is_ascii_control(), true);
    if never.len() < all {
        untie(text, from, never);
    }
    if never.is_empty() {
        return None;
    }
    // the text between, then the mask: no check of the copy's UTF-8, as bytes would need
    let mut copy = String::with_capacity(text.len());
    let mut from = 0;
    for &at in &*never {
        copy.push_str(&text[from..at]);
        copy.push(char::from(m));
        from = at + 1;
    }
    copy.push_str(&text[from..]);
    *source = Cow::Owned(copy);
    Some(m)
}

/// `never` (in order) less the `$`s on a link reference's line (`]:`, or the line after one
/// ending so), with `definitions`: a reference link's destination is its definition's, out of
/// the link's range, where [`Builder::rare_from`] wouldn't find it to unmask (a pass's false
/// closer, math's, is in none). Under a `control` byte, less those it would change the parse at,
/// where `$`, a punctuation byte, wouldn't: next to an emphasis or strikethrough delimiter run
/// ([`flanks_otherwise`]); in a link's destination, after a `](` not closed by a `)` (on its
/// line or the next, as one may go on); inside `<…>` on its line (an autolink). A destination
/// holds no control byte, so the link would break; HTML and code take one as they take `$`.
/// Reads only the lines with a `$` of `never`, and the line before each.
#[cold]
fn line_safe(b: &[u8], never: &mut Vec<usize>, control: bool, definitions: bool) {
    // the start of the line `seen` is of, `None` before the first; how far that line is read
    let (mut line, mut at) = (None, 0);
    let mut seen = LineSeen::default();
    never.retain(|&c| {
        let start = match line {
            Some(l) if memchr::memchr(b'\n', &b[at..c]).is_none() => l,
            _ => {
                let from = line.map_or(0, |_| at);
                memchr::memrchr(b'\n', &b[from..c]).map_or(from, |i| from + i + 1)
            }
        };
        if line != Some(start) {
            line = Some(start);
            seen = LineSeen::default();
            if start > 0 {
                // the line before: a definition's destination may come on the next line, as
                // may a link's
                let before = memchr::memrchr(b'\n', &b[..start - 1]).map_or(0, |i| i + 1);
                seen.read(b, before, start - 1);
                seen = LineSeen {
                    definition: b[before..start - 1].trim_ascii_end().ends_with(b"]:"),
                    dest: seen.dest,
                    angle: false,
                };
            }
            at = start;
        }
        seen.read(b, at, c);
        at = c;
        !(definitions && seen.definition
            || control && (seen.angle || seen.dest || flanks_otherwise(b, c)))
    });
}

/// What [`line_safe`] has read of a line.
#[derive(Default)]
struct LineSeen {
    /// A `]:`: a link reference's definition.
    definition: bool,
    /// A `](` without its `)`.
    dest: bool,
    /// A `<` without its `>`.
    angle: bool,
}

impl LineSeen {
    /// Read `b[from..to]`, on from what's read.
    fn read(&mut self, b: &[u8], from: usize, to: usize) {
        for i in from..to {
            let after_bracket = i > 0 && b[i - 1] == b']';
            match b[i] {
                b'<' => self.angle = true,
                b'>' => self.angle = false,
                b'(' if after_bracket => self.dest = true,
                b')' => self.dest = false,
                b':' if after_bracket => self.definition = true,
                _ => {}
            }
        }
    }
}

/// Whether a control byte in place of the `$` at `c` makes a delimiter run next to it flank
/// otherwise ([`line_safe`]): a control byte is no punctuation, as `$` is. A run (`*`, `_`,
/// `~`) followed by punctuation is left-flanking only after a blank or punctuation, and one
/// after punctuation is right-flanking only before a blank or punctuation. So one before the
/// `$` flanks the same either way only after a blank (or at a line's start), and one after it
/// (a backtick's `$` has no digit after it) may not. Linear: each run is read once.
fn flanks_otherwise(b: &[u8], c: usize) -> bool {
    if matches!(b.get(c + 1), Some(b'*' | b'_' | b'~')) {
        return true;
    }
    let d = b[c - 1];
    if !matches!(d, b'*' | b'_' | b'~') {
        return false;
    }
    let start = b[..c].iter().rposition(|&x| x != d).map_or(0, |i| i + 1);
    start > 0 && !blank_byte(b[start - 1])
}

/// The notice above an Excalidraw drawing's text (OBS-015).
const DRAWING: &str = "An Excalidraw drawing, shown as its text";

/// The most times [`markdown`] parses a text again for math closed on a `$` it shouldn't be:
/// each pass masks the first such `$` of each inline context, and those after it that masking
/// it leaves as they were ([`Builder::settle`]), so a context of false closers that each change
/// what follows takes a pass each. The last pass takes what it gets. 4 until PAR-016, when a
/// pass masked every one it met: random texts of markdown's bytes around `$` past it went from
/// 9.5% to 0.5% at 8; the vault's notes take 2 at most.
const MAX_MATH_PASSES: usize = 8;

/// Where the inline math at `range` (its `$`s included), of `tex` bytes of TeX, closes falsely,
/// its closing `$`, or `None` where it closes as pulldown-cmark closed it: its closing `$` not
/// followed by a digit (pandoc's rule), nor right after a backtick unless the math opened on one
/// (`` $`…`$ ``). pulldown-cmark's ranges reach past the `$` at a heading's end, over its blanks
/// (`# $a`$\t`), and math across a wiki link's `|` starts past the `|`, its opener left out
/// (`[[a $b|c$9]]`), so shorter than its TeX and `$`s, down to the closer alone (`[[$x|$4]]`):
/// its closer is judged alone, as math opened without a backtick.
fn wrong_closer(text: &str, range: &Range<usize>, tex: usize) -> Option<usize> {
    let b = text.as_bytes();
    let end = range.start + b[range.clone()].trim_ascii_end().len();
    let opener = range.len() >= tex + 2;
    let shortest = range.start + if opener { 3 } else { 1 };
    if end < shortest.max(2) || b[end - 1] != b'$' {
        return None;
    }
    let tick_closes = b[end - 2] != b'`' || (opener && b[range.start + 1] == b'`');
    let closes = !b.get(end).is_some_and(u8::is_ascii_digit) && tick_closes;
    (!closes).then_some(end - 1)
}

/// [`Builder::pending`]'s opener for a false closer whose math's opener isn't known (a wiki
/// link's target's, where pulldown-cmark's range for its math lacks one).
const NO_OPENER: usize = usize::MAX;

/// Whether masking the false closer at `at`, of math opened at `open`, in an inline context
/// ending by `end`, leaves the context's reading past it as it was ([`Builder::settle`]), so
/// that the false closers met past it are pandoc's too. pulldown-cmark pairs an opener with the
/// next `$` of its brace context: with `at` masked, the opener's math is undone if that `$` (the
/// next unmasked, unescaped one) can't close (a blank before it) or there's none, and in no
/// other case. Undone, its TeX is inline markdown again: so none of it may be syntax that can
/// reach past it, a code span, a link, a footnote's reference, inline HTML or an autolink. Nor
/// past it, as a failed opener leaves the context's `$`s in pulldown-cmark's queue of them,
/// which doesn't see a `$` a code span, a destination or HTML took after; past a closer that
/// queue is cleared. But a wiki link, whole on its line ([`wiki_pairs_only`]): one past `at`
/// reads as it did, and one the undone math frees reads back over nothing but its masked `$`s
/// (`$2 [[d$2]] and $3 [[e$3]]`, PAR-020). Braces between the opener and the next `$` may put
/// that `$` in another context: none either. Nor a `$` right before the opener: a `$$` that
/// opened no display math because its next `$` was the opener's false closer (`$$1x$5 $$`)
/// would pair with the next `$$` past it.
///
/// A false closer in a wiki link's target, whose `target` ends there (at its `|` or `]]`), has
/// its math in the target, which the link reads back over whatever its markup: undone, none of
/// it may reach past the target, a bracket neither (PAR-020).
fn reads_on(
    b: &[u8],
    masked: &[usize],
    open: usize,
    at: usize,
    target: Option<usize>,
    end: usize,
) -> bool {
    let end = end.min(b.len());
    if at >= end
        || (open > 0 && b[open - 1] == b'$')
        || (b[open + 1..end].iter()).any(|c| matches!(c, b'`' | b'<'))
    {
        return false;
    }
    let pairs = match target {
        None => wiki_pairs_only(b, masked, at, open + 1, end),
        Some(t) => {
            // past the link's `]]`, its text after a `|` holding no bracket
            let close = memchr::memmem::find(&b[t..end], b"]]").map(|i| t + i);
            close.is_some_and(|c| {
                !(b[open + 1..c].iter()).any(|c| matches!(c, b'[' | b']'))
                    && wiki_pairs_only(b, masked, at, c + 2, end)
            })
        }
    };
    if !pairs {
        return false;
    }
    let mut from = at + 1;
    let next = loop {
        match memchr::memchr(b'$', &b[from..end]).map(|i| from + i) {
            Some(d) if masked.binary_search(&d).is_ok() || escaped(b, d) => from = d + 1,
            other => break other,
        }
    };
    !(b[open + 1..next.unwrap_or(end)].iter()).any(|c| matches!(c, b'{' | b'}'))
        && next.is_none_or(|d| blank_byte(b[d - 1]))
}

/// Whether the brackets of `b[from..to]` are all wiki links' ([`reads_on`]): `[[`, then `]]`
/// before any other bracket or a line's end, and inside, if it opens before the false closer at
/// `at` (in the math its mask undoes), no `$` but `at` and those `masked`.
fn wiki_pairs_only(b: &[u8], masked: &[usize], at: usize, from: usize, to: usize) -> bool {
    let mut i = from;
    while let Some(j) = memchr::memchr2(b'[', b']', &b[i..to]).map(|k| i + k) {
        if b[j] == b']' || b.get(j + 1) != Some(&b'[') {
            return false;
        }
        let inner = j + 2;
        let Some(close) = memchr::memchr3(b'[', b']', b'\n', &b[inner..to]).map(|k| inner + k)
        else {
            return false;
        };
        if b[close] != b']' || b.get(close + 1) != Some(&b']') || close + 2 > to {
            return false;
        }
        let freed = j < at
            && memchr::memchr_iter(b'$', &b[inner..close])
                .map(|k| inner + k)
                .any(|d| d != at && masked.binary_search(&d).is_err());
        if freed {
            return false;
        }
        i = close + 2;
    }
    true
}

/// Whether a `tag` is inline: else it's a block's, and its start ends an inline context
/// ([`Builder::settle`]).
fn inline_tag(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

/// [`inline_tag`] for a tag's end.
fn inline_end(end: TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

/// A byte `text` doesn't hold, to mask a `$` with. First ASCII punctuation that means nothing to
/// CommonMark, GFM or the extensions on wherever a `$` of `text` stands, as `$` (outside math)
/// doesn't: punctuation, as `$` is, so emphasis next to it flanks the same, and a link's
/// destination may hold it. `%` means nothing anywhere; `?` opens a processing instruction after
/// a `<` (`<?`), `^` a footnote's reference after a `[` (`[^`), and inside `<…>` a `,` ends an
/// email autolink's local part and an `@` makes one, so each is taken only where no `$` of
/// `text` stands so. Then, rarer still in a text holding those five, as a mask only where no `$`
/// it may mask stands so ([`maskable_spots`]; PAR-018): `/` (no meaning but in an HTML tag's
/// `/>` or `</`, which a `$` before a digit or after a backtick can't make); `#` (after an `&`
/// it starts a numeric entity); `;` (after an entity's name it ends one, and an email autolink's
/// local part can't hold it); `+` and `.` (an autolink's scheme or an email autolink's domain
/// may hold them, inside `<…>`). Not `-`, which ends an HTML comment or block (`` `$-> ``).
/// Not a byte the text writes as an entity either (`&#37;`, `&percnt;`), which pulldown-cmark
/// decodes into the event's strings, where it would be taken for a masked `$`. Then a control
/// byte, which a link's destination may not hold (a masked `$` in one breaks the link, as rare
/// as a text holding all ten): not a tab, line feed, form feed or carriage return, nor NUL,
/// which pulldown-cmark replaces.
fn unused_byte(text: &str) -> Option<u8> {
    let b = text.as_bytes();
    let entities = Entities::new(b);
    let free = |c: u8| memchr::memchr(c, b).is_none() && !entities.writes(c);
    let after = |c: u8| memchr::memmem::find(b, &[c, b'$']).is_some();
    let angles = || memchr::memchr(b'<', b).is_some();
    let punct = if free(b'%') {
        Some(b'%')
    } else if free(b'?') && !after(b'<') {
        Some(b'?')
    } else if free(b'^') && !after(b'[') {
        Some(b'^')
    } else if free(b',') && !angles() {
        Some(b',')
    } else if free(b'@') && !angles() {
        Some(b'@')
    } else {
        rarer_byte(b, free)
    };
    punct.or_else(|| (1..32u8).find(|&c| !matches!(c, b'\t'..=b'\r') && free(c)))
}

/// [`unused_byte`]'s punctuation past its first five, `free` the bytes the text neither holds
/// nor writes as entities.
#[cold]
fn rarer_byte(b: &[u8], free: impl Fn(u8) -> bool) -> Option<u8> {
    (rarer_bytes(&maskable_spots(b)).into_iter())
        .find(|&(c, ok)| ok && free(c))
        .map(|(c, _)| c)
}

/// [`rarer_byte`]'s bytes, in order, each with whether no `$` stands where it would mean
/// something ([`unused_byte`]).
fn rarer_bytes(spots: &Spots) -> [(u8, bool); 6] {
    [
        (b'/', true),
        (b'#', !spots.after_amp),
        (b'-', !spots.angle),
        (b'+', !spots.angle),
        (b'.', !spots.angle),
        (b';', !spots.after_name && !spots.angle),
    ]
}

/// Where the `$`s that may be masked ([`markdown`]) stand in a text, for [`rarer_byte`]: a `$`
/// not escaped, after a byte that isn't blank, before a digit or after a backtick, as every
/// false closer and price is (a label's copies too, the same text).
#[derive(Default)]
struct Spots {
    /// One right after an `&`.
    after_amp: bool,
    /// One right after an entity's name (`&amp`, `&#12`).
    after_name: bool,
    /// One after a `<` on its line, with no `>` between.
    angle: bool,
}

/// [`Spots`] of `b`: linear, each byte read once or twice.
fn maskable_spots(b: &[u8]) -> Spots {
    let mut s = Spots::default();
    // whether a `<` is open on the line before the last `$` read, and how far that's read
    let (mut open, mut read) = (false, 0);
    for c in memchr::memchr_iter(b'$', b) {
        if let Some(i) = memchr::memrchr3(b'<', b'>', b'\n', &b[read..c]) {
            open = b[read + i] == b'<';
        }
        read = c;
        if c == 0 || blank_byte(b[c - 1]) || escaped(b, c) {
            continue;
        }
        if b[c - 1] != b'`' && !b.get(c + 1).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        s.after_amp |= b[c - 1] == b'&';
        // a name stops at the `$` before
        let n = (b[..c].iter().rev())
            .take_while(|x| x.is_ascii_alphanumeric() || **x == b'#')
            .count();
        s.after_name |= n > 0 && c > n && b[c - n - 1] == b'&';
        s.angle |= open;
    }
    s
}

/// The ASCII bytes a text writes as entities, which pulldown-cmark decodes ([`unused_byte`]):
/// numeric ones, and the mask bytes' HTML names. One pass over the text's `&`s.
struct Entities(u128);

impl Entities {
    fn new(b: &[u8]) -> Self {
        let mut set = 0u128;
        for at in memchr::memchr_iter(b'&', b) {
            let rest = &b[at + 1..];
            let byte = match rest.first() {
                // CommonMark's: `&#` and 1 to 7 digits, or `&#x` and 1 to 6 hex digits, then `;`
                Some(b'#') => {
                    let (radix, digits) = match rest.get(1) {
                        Some(b'x' | b'X') => (16, &rest[2..]),
                        _ => (10, &rest[1..]),
                    };
                    let n = (digits.iter())
                        .take_while(|c| c.is_ascii_digit() || radix == 16 && c.is_ascii_hexdigit())
                        .count();
                    if n == 0 || n > 7 || digits.get(n) != Some(&b';') {
                        continue;
                    }
                    let digits = std::str::from_utf8(&digits[..n]).expect("ASCII digits");
                    u32::from_str_radix(digits, radix).ok()
                }
                _ => {
                    let n = rest
                        .iter()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .count();
                    if rest.get(n) != Some(&b';') {
                        continue;
                    }
                    let c = match &rest[..n] {
                        b"percnt" => b'%',
                        b"quest" => b'?',
                        b"Hat" => b'^',
                        b"comma" => b',',
                        b"commat" => b'@',
                        b"sol" => b'/',
                        b"num" => b'#',
                        b"semi" => b';',
                        b"plus" => b'+',
                        b"period" => b'.',
                        _ => continue,
                    };
                    Some(u32::from(c))
                }
            };
            if let Some(v) = byte.filter(|&v| v < 128) {
                set |= 1 << v;
            }
        }
        Self(set)
    }

    /// Whether the text writes `c` as an entity.
    fn writes(&self, c: u8) -> bool {
        c < 128 && self.0 & (1 << c) != 0
    }
}

/// `event` with each masked `$` written back. A `$` masked before the first pass can stand
/// anywhere ([`never_opens`]); one a pass masked, where math stood in an earlier pass, inline. So
/// it can land in text, code, math, HTML, a footnote's label or a link's parts, and a fence's
/// info string.
#[cold]
#[inline(never)]
fn unmask(event: Event<'_>, m: u8) -> Event<'_> {
    let un = |s| unmasked(s, m);
    match event {
        Event::Text(s) => Event::Text(un(s)),
        Event::Code(s) => Event::Code(un(s)),
        Event::InlineMath(s) => Event::InlineMath(un(s)),
        Event::DisplayMath(s) => Event::DisplayMath(un(s)),
        Event::Html(s) => Event::Html(un(s)),
        Event::InlineHtml(s) => Event::InlineHtml(un(s)),
        Event::FootnoteReference(s) => Event::FootnoteReference(un(s)),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: un(dest_url),
            title: un(title),
            id: un(id),
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: un(dest_url),
            title: un(title),
            id: un(id),
        }),
        Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(un(info))))
        }
        Event::Start(Tag::FootnoteDefinition(label)) => {
            Event::Start(Tag::FootnoteDefinition(un(label)))
        }
        other => other,
    }
}

/// `s` with each `$` masked with `m` written back.
fn unmasked(s: CowStr<'_>, m: u8) -> CowStr<'_> {
    if s.as_bytes().contains(&m) {
        s.replace(char::from(m), "$").into()
    } else {
        s
    }
}

/// The most quotes and lists nested in each other. Deeper ones fold into the last, as
/// [`Container::Other`] does: layout, search and selection walk the blocks recursively, and
/// 10,000 `>` (10 KB) overflowed the main thread's stack (F-012). 100 levels take 200 columns
/// of bars before any text; markdown-it stops at 100 too (`maxNesting`).
const MAX_NESTING: usize = 100;

/// The line (from 1) `offset` is on, sought forward from `line`, which starts at or before it: a
/// gallop, so the next line costs a comparison and a line `n` further a search over `2n`.
fn line_from(starts: &[u32], line: u32, offset: usize) -> u32 {
    // the first `base` lines start at or before `offset`
    let mut base = line as usize;
    let mut step = 1;
    loop {
        let probe = base + step - 1;
        if probe >= starts.len() || starts[probe] as usize > offset {
            let end = probe.min(starts.len());
            return (base + starts[base..end].partition_point(|&s| s as usize <= offset)) as u32;
        }
        base = probe + 1;
        step *= 2;
    }
}

enum Container {
    Root,
    Quote(Option<Callout>),
    Item,
    Footnote(FootnoteId),
    /// Anything we flatten into its parent (definition lists, which are not enabled).
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineKind {
    Paragraph,
    /// Text directly inside a container: the content of a tight list item.
    Implicit,
    Heading(u8),
    Cell,
    /// The first line of a callout's paragraph, its title, up to the line's break (OBS-002).
    Title,
}

/// A callout's marker, `[!kind]` and a fold sign, on the first line of a quote just opened
/// (OBS-002). pulldown-cmark reads GitHub's five kinds alone on their line; the rest are text.
struct Marker {
    kind: CalloutKind,
    /// The kind as written.
    name: Range<usize>,
    fold: Option<Fold>,
    /// Where the `[` is: the quote is a callout when its paragraph starts here.
    at: usize,
    /// Past the `]` and the fold sign.
    end: usize,
    /// Where the title starts, past the spaces after `end`.
    title: usize,
    state: MarkerState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkerState {
    /// The quote opened; its first event tells.
    Armed,
    /// Its paragraph opened as the title; the marker's text, up to `upto`, is dropped.
    Reading { upto: usize },
    /// The marker was read whole: the open title is the callout's.
    Title,
}

enum Frame {
    Blocks {
        kind: Container,
        blocks: Vec<Block>,
        src: LineRange,
        task: Option<Task>,
    },
    List {
        start: Option<u64>,
        loose: bool,
        items: Vec<ListItem>,
        src: LineRange,
    },
    /// Its runs are [`Builder::runs`] from `start` on.
    Inline {
        kind: InlineKind,
        start: u32,
        src: LineRange,
    },
    Code {
        lang: Option<Box<str>>,
        text: String,
        src: LineRange,
        fenced: bool,
    },
    Html {
        text: String,
        src: LineRange,
    },
    Table {
        aligns: Vec<Align>,
        header: TableRow,
        rows: Vec<TableRow>,
        /// Every row's cells so far, the open row's from `row_start` on.
        cells: Vec<Inlines>,
        row_start: u32,
        row_src: LineRange,
        src: LineRange,
    },
    /// A metadata block, skipped; pulldown-cmark makes none, its option off (`front::find`).
    Skip,
}

/// An image's alt text is written into [`Builder::text`] from `start` on as it comes.
struct ImageDraft {
    link: LinkId,
    start: u32,
    line: u32,
    /// An embed of a note or another file that isn't an image (OBS-007): a link, not an image.
    embed: bool,
    /// A wiki embed with a `|`: its alt text may be a size alone (`![[pic.png|100]]`, OBS-008).
    aliased: bool,
}

struct Builder<'s> {
    /// The text parsed, unmasked.
    source: &'s str,
    line_starts: Vec<u32>,
    /// Every run's text, in source order: the [`Doc::text`] the runs' ranges are of (PAR-004).
    text: String,
    /// Every closed paragraph's, heading's and cell's runs, then the open ones' (PAR-008).
    runs: Vec<Run>,
    /// The inline frames on the stack, so closing one needn't look for others under it: the
    /// look ran at every paragraph and cell, 0.4–1.5% of parse's instructions (experiment 76).
    inlines_open: u32,
    /// The line the last event started on, from 1.
    line: u32,
    stack: Vec<Frame>,
    links: Vec<LinkRef>,
    anchors: Vec<Anchor>,
    footnotes: Vec<Footnote>,
    footnote_ids: HashMap<Box<str>, FootnoteId>,
    ref_order: Vec<FootnoteId>,
    slugs: Slugger,
    style: InlineStyle,
    link: Option<LinkId>,
    image: Option<ImageDraft>,
    tables: TableId,
    code_blocks: CodeId,
    units: UnitId,
    /// Quotes and lists open on the stack, up to [`MAX_NESTING`].
    nesting: usize,
    /// Obsidian's line breaks: a single newline breaks the line (OBS-001, ADR-45).
    line_breaks: bool,
    /// The open paragraph is the rest of one display math split (OBS-003), with no text yet: the
    /// line break that ended the math's line doesn't start it, and its lines start at its text.
    after_math: bool,
    /// The note holds a `$` past its front matter: only then may a wiki link's target hide math
    /// ([`Builder::wiki_target`]).
    dollars: bool,
    /// Obsidian's dialect: GitHub's `important` and `caution` are its tip and warning (OBS-002).
    obsidian: bool,
    /// What opens the quote or item just opened, being read: a callout's marker (OBS-002) or a
    /// task status past GFM's (OBS-012). One field, so an event pays one test for both.
    opening: Option<Opening>,
    /// The byte this pass's source has its false closers' `$`s masked with ([`markdown`]):
    /// the strings of an event at or past one get them back, in [`Builder::event_rare`].
    mask: Option<u8>,
    /// Where the source's `$`s are masked, in order: before the first pass ([`never_opens`])
    /// and by the passes before this one.
    masked: Vec<usize>,
    /// The first of [`Builder::masked`] past the last event that went to
    /// [`Builder::event_rare`] (`usize::MAX` past the last): an event ending at or before it
    /// holds none, so a note with a masked `$` sends only the events around each the slow way.
    /// Events start in source order (ends repeat their starts, and hold no string), but around
    /// a wiki link ([`Builder::wiki_hold`]); an event's strings are of its range, but for math
    /// across a wiki link's `|`, whose range starts past the `|` and ends past the `$`, and a
    /// reference link's destination and title, its definition's, given back as the link is added
    /// ([`Builder::add_masked_link`]).
    rare_from: usize,
    /// Where a wiki link (or embed) holding a masked `$` starts, in the inline context being
    /// read, else `usize::MAX`: [`Builder::rare_from`] stays there until the context ends, so
    /// every event ending past it goes the slow way. pulldown-cmark may give the events around
    /// one out of order: an inner image's start (`![[![^a$1]/$6 … [[$x|](">1. )…]]`, at the
    /// `](`) before the text and the math at the `$6`, or, where an image and the link overlap
    /// (`![[[a$1]: … ](]() … [^a$1]$4c$1]]`), the image's text past the link's end. Moved past a
    /// mask by the later event, the earlier kept it.
    wiki_hold: usize,
    /// Where the false closers of inline math this pass met are, their `$`s ([`markdown`]),
    /// those pandoc's rule skips as this pass reads them ([`Builder::settle`]): the next pass
    /// masks them all.
    wrong: Vec<usize>,
    /// The false closers met in the inline context being read (a paragraph's, a heading's, a
    /// cell's), in order, each with where its math opened ([`NO_OPENER`] where that isn't
    /// known) and, in a wiki link's target, where the target ends, for [`Builder::settle`] at
    /// the context's end.
    pending: Vec<(usize, usize, Option<usize>)>,
    /// Where false closers left unmasked under a control byte are, in order: their math is
    /// read as pulldown-cmark reads it, as by the last pass ([`kept_besides`], PAR-015).
    kept: Vec<usize>,
    /// The pass met a false closer: it will be parsed again, so the builder took no event
    /// since, and its events only go through [`Builder::event_rare`] for their false closers
    /// and where inline contexts end ([`Builder::settle`]) (a builder that read on built the
    /// whole note to throw it away).
    dead: bool,
    /// The pass takes its math as pulldown-cmark reads it: it neither stops at a false closer
    /// nor is parsed again ([`MAX_MATH_PASSES`]).
    last: bool,
    /// Where each run of two or more `=` in the text has its second: a closing frame's runs may
    /// hold a highlight only when one is on their lines (OBS-005).
    marks: Vec<u32>,
    /// The first of [`Builder::marks`] at or past the last frame [`Builder::marked`] looked at.
    mark: usize,
    /// The lines of the marks either side of [`Builder::mark`]: the last before it (0 for none)
    /// and the first at or past it (`u32::MAX` for none). A frame on lines between them holds
    /// none, which [`Builder::close_runs`] tells in two compares: the search a frame cost XL's
    /// parse +1.8% instructions (PAR-022).
    mark_lines: (u32, u32),
    /// Where each escaped `#` (`\#word`) is in [`Builder::text`], in Obsidian's dialect: no tag
    /// opens there (OBS-006).
    escaped: Vec<u32>,
    /// Typeset math's TeX (OBS-003), [`Doc::math`].
    math: Vec<TextRange>,
    /// Where embeds are transcluded from (OBS-007); `None` leaves them links.
    embedding: Option<Embedding<'s>>,
    /// The notes transcluded, [`Doc::embedded`].
    embedded: Vec<Embedded>,
    /// The code pulldown-cmark finds, as [`super::comments::code`] does, in the first pass over
    /// an Obsidian note whose `%%`s all look like code ([`passes`], PAR-021); else `None`. That
    /// pass sends every event through [`Builder::event_rare`] ([`Builder::rare_from`] held at 0),
    /// which finds it: a test in [`Builder::dispatch`]'s arms cost every note's parse +0.2 to
    /// +0.6% instructions.
    code: Option<Vec<Range<usize>>>,
    /// A builder that builds nothing, its events looked at by this ([`probe`]); else `None`. Sent
    /// every event through [`Builder::event_rare`] as [`Builder::code`]'s pass is.
    probe: Option<Probe>,
}

impl<'s> Builder<'s> {
    #[cfg(test)]
    fn new(text: &'s str) -> Self {
        let Scan { starts, marks, .. } = super::lines::scan(text, 0);
        Self::with_lines(text, starts, marks)
    }

    /// A builder of `text`, whose line starts and marks are `line_starts` and `marks`
    /// ([`super::lines::scan`]'s, found with whether `text` holds a `$`).
    fn with_lines(text: &'s str, line_starts: Vec<u32>, marks: Vec<u32>) -> Self {
        // the source's length, which inline text rarely passes: grown by doubling instead, a
        // long run's text was copied at each step (long lines' parse +5–10%, experiment 71);
        // the room left over is never touched, and `finish` gives it back
        Self::with_room(text, line_starts, marks, text.len())
    }

    /// A builder of `text` that builds nothing, its every event looked at by `probe` ([`probe`]):
    /// one line, no room for inline text.
    fn probing(text: &'s str, probe: Probe) -> Self {
        let mut b = Self::with_room(text, vec![0], Vec::new(), 0);
        b.rare_from = 0;
        b.probe = Some(probe);
        b
    }

    /// [`Builder::with_lines`], its inline text's room `room`.
    fn with_room(text: &'s str, line_starts: Vec<u32>, marks: Vec<u32>, room: usize) -> Self {
        let mut b = Self {
            source: text,
            line_starts,
            text: String::with_capacity(room),
            runs: Vec::new(),
            inlines_open: 0,
            line: 1,
            stack: vec![Frame::Blocks {
                kind: Container::Root,
                blocks: Vec::new(),
                src: LineRange::NONE,
                task: None,
            }],
            links: Vec::new(),
            anchors: Vec::new(),
            footnotes: Vec::new(),
            footnote_ids: HashMap::new(),
            ref_order: Vec::new(),
            slugs: Slugger::default(),
            style: InlineStyle::NONE,
            link: None,
            image: None,
            tables: 0,
            code_blocks: 0,
            units: 0,
            nesting: 0,
            line_breaks: false,
            after_math: false,
            dollars: false,
            obsidian: false,
            opening: None,
            mask: None,
            masked: Vec::new(),
            rare_from: usize::MAX,
            wiki_hold: usize::MAX,
            wrong: Vec::new(),
            pending: Vec::new(),
            kept: Vec::new(),
            dead: false,
            last: false,
            marks,
            mark: 0,
            mark_lines: (0, u32::MAX),
            escaped: Vec::new(),
            math: Vec::new(),
            embedding: None,
            embedded: Vec::new(),
            code: None,
            probe: None,
        };
        b.mark_lines = b.lines_beside(0);
        b
    }

    /// The line an event starts on, sought from the last event's: events come in source order,
    /// nearly, so most are on its line or the next (PAR-002).
    fn line_of(&mut self, offset: usize) -> u32 {
        self.line = if self.line_starts[self.line as usize - 1] as usize <= offset {
            line_from(&self.line_starts, self.line, offset)
        } else {
            self.line_starts
                .partition_point(|&s| s as usize <= offset)
                .max(1) as u32
        };
        self.line
    }

    /// Inlined into [`read_pass`], its one caller: what it inlines in turn grew it past what
    /// LLVM inlines unasked (PAR-010).
    #[inline(always)]
    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        let line = self.line_of(range.start);
        let src = LineRange::new(
            line,
            line_from(
                &self.line_starts,
                line,
                range.end.saturating_sub(1).max(range.start),
            ),
        );
        // the search the cursor replaced, in debug builds (the fuzzers', `cargo test`)
        debug_assert_eq!(src, {
            let search = |o: usize| self.line_starts.partition_point(|&s| s as usize <= o) as u32;
            LineRange::new(
                search(range.start),
                search(range.end.saturating_sub(1).max(range.start)),
            )
        });
        // the event goes on as pulldown-cmark left it: a copy of it, to rewrite in place or to give
        // back from a `match`, stalled on the iterator's stores, every note's parse +10% CPU
        if range.end > self.rare_from || self.opening.is_some() || self.dead {
            return self.event_rare(event, range, src, line);
        }
        self.dispatch(event, range, src, line);
    }

    /// [`Builder::event`]'s work on `event` once its lines are known. Inlined into both of its
    /// callers, [`Builder::event`] and [`Builder::event_rare`], as are the builder's calls it
    /// inlines (`start`, `end`, `text`): a call of any of them took it out of line (PAR-010).
    #[inline(always)]
    fn dispatch(&mut self, event: Event<'_>, range: Range<usize>, src: LineRange, line: u32) {
        match event {
            // one call of `start` and one of `text`, each inlined here: a second call of either
            // (an arm of a quote's own, of a tag's) took it out of line (PAR-010)
            Event::Start(tag) => self.start(tag, src, range),
            Event::End(end) => self.end(end),
            Event::Text(t) => {
                // the length, not `text_end`: its call, out of line in `read_pass`, cost every
                // text event (PAR-022); `escaped_hash` checks it
                let at = self.text.len();
                self.text(&t, line);
                if self.obsidian && t.starts_with('#') {
                    self.escaped_hash(at, &range);
                }
            }
            Event::Code(t) => self.push_text(&t, self.style | InlineStyle::CODE, line),
            // typeset as Unicode, else the TeX as written, as code (OBS-003)
            Event::InlineMath(t) => {
                if let Some(at) = wrong_closer(self.source, &range, t.len()) {
                    self.false_closer(at);
                }
                match tex::inline(&t) {
                    Some(math) => self.push_math(&math, &t, line),
                    None => self.push_text(&t, self.style | InlineStyle::CODE, line),
                }
            }
            Event::DisplayMath(t) => self.display_math(&t, src),
            Event::Html(t) => match self.stack.last_mut() {
                Some(Frame::Html { text, .. }) => text.push_str(&t),
                _ => self.inline_html(&t, line),
            },
            Event::InlineHtml(t) => self.inline_html(&t, line),
            Event::FootnoteReference(label) => self.footnote_ref(&label, line),
            Event::SoftBreak if self.line_breaks => self.push_break(RunKind::HardBreak, line),
            Event::SoftBreak => self.push_break(RunKind::SoftBreak, line),
            Event::HardBreak => self.push_break(RunKind::HardBreak, line),
            Event::Rule => self.push_block(Block {
                kind: BlockKind::Rule,
                src,
            }),
            Event::TaskListMarker(done) => self.task(match done {
                true => Task::Done,
                false => Task::Open,
            }),
        }
    }

    /// A `#` text at `range` written to [`Builder::text`] from `at`, in Obsidian's dialect:
    /// `\#word` is no tag (OBS-006), and pulldown-cmark makes the escaped `#` text of its own.
    #[inline(never)]
    fn escaped_hash(&mut self, at: usize, range: &Range<usize>) {
        let at = u32::try_from(at).expect("inline text under 4 GiB");
        let b = self.source.as_bytes();
        let escaped = b.get(range.start) == Some(&b'\\')
            || range.start.checked_sub(1).and_then(|i| b.get(i)) == Some(&b'\\');
        if escaped && self.text_end() > at {
            // any past it are of text taken back since (a run with nowhere to go)
            while self.escaped.last().is_some_and(|&e| e >= at) {
                self.escaped.pop();
            }
            self.escaped.push(at);
        }
    }

    /// [`Builder::event`] of `event` with its masked `$`s written back ([`Builder::mask`]),
    /// and past what opens a quote or an item, being read ([`Builder::opening`]). Out of line:
    /// an event pays a test for both. An event at or past [`Builder::rare_from`] moves it on.
    #[inline(never)]
    fn event_rare(&mut self, event: Event<'_>, range: Range<usize>, src: LineRange, line: u32) {
        // a probe's builder only looks ([`probe`], PAR-023)
        if let Some(probe) = &mut self.probe {
            probe.event(self.source, &event, range);
            return;
        }
        // the first pass's code (PAR-021), its every event sent here ([`Builder::code`])
        if let Some(code) = &mut self.code
            && super::comments::is_code(&event)
        {
            code.push(range.clone());
        }
        if self.dead {
            #[cfg(test)]
            if tests::ideal() {
                return;
            }
            // the rest of the pass's false closers, an inline context's settled at its end
            match &event {
                Event::InlineMath(t) => {
                    if let Some(at) = wrong_closer(self.source, &range, t.len()) {
                        self.false_closer(at);
                    }
                }
                Event::Start(
                    Tag::Link {
                        link_type: LinkType::WikiLink { .. },
                        dest_url,
                        ..
                    }
                    | Tag::Image {
                        link_type: LinkType::WikiLink { .. },
                        dest_url,
                        ..
                    },
                ) => {
                    // as written: this event's strings aren't given back their `$`s
                    let dest = match self.mask {
                        Some(m) => unmasked(dest_url.clone(), m),
                        None => dest_url.clone(),
                    };
                    if memchr::memchr(b'$', dest.as_bytes()).is_some() {
                        self.wiki_target(&range, &dest);
                    }
                }
                // a block's start or end ends the inline context being read
                Event::Start(tag) if !inline_tag(tag) => self.settle(range.start),
                Event::End(end) if !inline_end(*end) => self.settle(range.end),
                _ => {}
            }
            return;
        }
        let event = match self.mask {
            Some(m) => {
                match &event {
                    Event::End(end) => {
                        // the inline context's end: the events past it are in order
                        if self.wiki_hold != usize::MAX && !inline_end(*end) {
                            let next = self.masked.partition_point(|&at| at < self.wiki_hold);
                            self.rare_from = self.masked.get(next).copied().unwrap_or(usize::MAX);
                            self.wiki_hold = usize::MAX;
                        }
                    }
                    _ => {
                        let next = self.masked.partition_point(|&at| at < range.start);
                        let at = self.masked.get(next).copied().unwrap_or(usize::MAX);
                        if self.wiki_hold == usize::MAX
                            && at < range.end
                            && matches!(
                                event,
                                Event::Start(
                                    Tag::Link {
                                        link_type: LinkType::WikiLink { .. },
                                        ..
                                    } | Tag::Image {
                                        link_type: LinkType::WikiLink { .. },
                                        ..
                                    }
                                )
                            )
                        {
                            self.wiki_hold = range.start;
                        }
                        self.rare_from = at.min(self.wiki_hold);
                    }
                }
                if self.code.is_some() {
                    self.rare_from = 0;
                }
                unmask(event, m)
            }
            None => event,
        };
        let event = match self.opening {
            None => Some(event),
            Some(Opening::Callout(_)) => self.read_marker(event, &range, src),
            Some(Opening::Status(_)) => self.read_status(event, &range),
        };
        if let Some(event) = event {
            self.dispatch(event, range, src, line);
        }
    }

    /// A false closer of inline math, its `$` at `at`: the pass's builder stops at the first,
    /// its doc to be parsed again with the `$` masked, unless the pass is the last
    /// ([`Builder::last`]) or the false closer is kept ([`Builder::kept`]). Its math opened at
    /// the last `$` before it, unmasked and unescaped: no other is between, but one in other
    /// braces, which [`reads_on`] doesn't take (math across a wiki link's `|`, whose range
    /// starts past it, too). Found here, not from the event's range: an argument more from the
    /// pass's loop cost every note's parse +0.2 to +0.5% instructions (PAR-016).
    #[cold]
    #[inline(never)]
    fn false_closer(&mut self, at: usize) {
        let b = self.source.as_bytes();
        let mut to = at;
        let open = loop {
            match memchr::memrchr(b'$', &b[..to]) {
                Some(o) if self.masked.binary_search(&o).is_ok() || escaped(b, o) => to = o,
                Some(o) => break o,
                None => break NO_OPENER,
            }
        };
        self.met(at, open, None);
    }

    /// [`Builder::false_closer`]'s work, and a wiki link's target's ([`Builder::wiki_target`],
    /// the target ending at `target`), whose math opened at `open`: the false closer at `at`
    /// waits for its inline context's end ([`Builder::settle`]).
    fn met(&mut self, at: usize, open: usize, target: Option<usize>) {
        if self.kept.binary_search(&at).is_ok() {
            return;
        }
        if self.last {
            #[cfg(test)]
            tests::CAPPED.set(true);
            return;
        }
        self.dead = true;
        self.pending.push((open, at, target));
    }

    /// The false closers met in the inline context that ends at `end` ([`Builder::pending`]) to
    /// mask, as pandoc's rule skips them: its first, and each after one that, masked, leaves the
    /// context's reading past it as it was ([`reads_on`]); the rest dropped. Masked all, a later
    /// one may be a closer only of math the earlier mask undoes, and stay masked where pandoc's
    /// rule would open math at it (PAR-016); masked one a pass, a context of `n` takes `n`
    /// passes. A wiki link's target's are among them, read on past as the others are (PAR-020;
    /// before, each stopped its context's run, one pass a target). Contexts are read apart
    /// (pulldown-cmark parses each leaf block's inlines alone), so each context's first is
    /// pandoc's whatever the others' masks; a label shared between them is [`label_copies`]'
    /// and [`stepwise`]'s to keep in step.
    #[cold]
    #[inline(never)]
    fn settle(&mut self, end: usize) {
        let b = self.source.as_bytes();
        for &(open, at, target) in &self.pending {
            self.wrong.push(at);
            if open == NO_OPENER || !reads_on(b, &self.masked, open, at, target, end) {
                break;
            }
        }
        self.pending.clear();
    }

    /// The false closers of math in the target of the wiki link (or embed) at `range`, met as
    /// [`Builder::false_closer`] meets one in an event: pulldown-cmark keeps a target's text
    /// whole, so math that opens and closes in it makes no event (PAR-014, `[[a $b c$1]] d$`,
    /// whose `$b` reaches the `d$` with the `$1` masked, and the math crosses the link).
    /// The target, as this pass parsed it, is parsed alone, after a letter so that no block
    /// opens: math open before a wiki link and closing in it or past it would have taken the
    /// link, so the target's `$`s pair among themselves. Only a target holding a `$` gets here.
    /// The target is `dest`, with its `$`s, found in the link's text: past its `[[`, or where
    /// pulldown-cmark took it from, the last of it in the range (it may start further on, past
    /// markup it split at: `![[\t|2{… }$41. *[^a$1]:]]`, whose target is `}$41. *[^a$1]:`).
    #[cold]
    #[inline(never)]
    fn wiki_target(&mut self, range: &Range<usize>, dest: &str) {
        let b = self.source.as_bytes();
        let opens = range.start + if b[range.start] == b'!' { 3 } else { 2 };
        let start = match b[range.clone()].get(opens - range.start..) {
            Some(rest) if rest.starts_with(dest.as_bytes()) => opens,
            _ => match memchr::memmem::rfind(&b[range.clone()], dest.as_bytes()) {
                Some(i) => range.start + i,
                None => return,
            },
        };
        let end = start + dest.len();
        let mut target = Vec::with_capacity(end - start + 1);
        target.push(b'a');
        target.extend_from_slice(&b[start..end]);
        if let Some(m) = self.mask {
            let from = self.masked.partition_point(|&at| at < start);
            for &at in self.masked[from..].iter().take_while(|&&at| at < end) {
                target[at - start + 1] = m;
            }
        }
        let Ok(target) = String::from_utf8(target) else {
            return;
        };
        // a parse of the target's own, nested in this one's ([`probe`]: the parse stays the
        // one reader of pulldown-cmark's events)
        let (Probe::Wiki(wrong), _) = probe(&target, 0, usize::MAX, Probe::Wiki(Vec::new())) else {
            unreachable!("a probe gives back its own kind");
        };
        for (r, at) in wrong {
            // the reference masks a pass's first false closer only
            #[cfg(test)]
            if self.dead && tests::ideal() {
                return;
            }
            // its opener, in the target as in the link's context ([`Builder::settle`]
            // reads on past it where masking it leaves the rest as it was, PAR-020)
            let open = match target.as_bytes()[r.start] {
                b'$' if r.start > 0 => start + r.start - 1,
                _ => NO_OPENER,
            };
            self.met(start + at - 1, open, Some(end));
        }
    }

    /// Make the innermost container a task, unless it's an item folded into its parent.
    fn task(&mut self, kind: Task) {
        let item = self.stack.iter_mut().rev().find_map(|f| match f {
            Frame::Blocks { kind, task, .. } => {
                Some(matches!(kind, Container::Item).then_some(task))
            }
            _ => None,
        });
        if let Some(Some(task)) = item {
            *task = Some(kind);
        }
    }

    /// An item's task status past GFM's (OBS-012), which pulldown-cmark reads as text split at
    /// its brackets: the item is a task when its first text starts at the `[`, and the status's
    /// text goes, with the blanks after it. Any other event first leaves the item as it is.
    fn read_status<'e>(&mut self, event: Event<'e>, range: &Range<usize>) -> Option<Event<'e>> {
        let Some(Opening::Status(s)) = &mut self.opening else {
            return Some(event);
        };
        match event {
            // a loose item's paragraph opens first
            Event::Start(Tag::Paragraph) if !s.read => Some(event),
            // ASCII without escapes or entities up to `end`: the text's offsets are the source's
            Event::Text(t) if range.start == s.next && t.len() == range.len() => {
                let (task, end) = (s.task, s.end);
                if !mem::replace(&mut s.read, true) {
                    self.task(task);
                }
                let Some(Opening::Status(s)) = &mut self.opening else {
                    return None;
                };
                if range.end <= end {
                    s.next = range.end;
                    return None;
                }
                self.opening = None;
                let rest = t[end - range.start..].trim_start_matches([' ', '\t']);
                (!rest.is_empty()).then(|| Event::Text(rest.to_owned().into()))
            }
            _ => {
                self.opening = None;
                Some(event)
            }
        }
    }

    /// The start of `tag`, at `range`: a quote's and an item's arm read what opens them, a
    /// callout's marker or a task status past GFM's ([`Builder::opening`]).
    #[inline(always)]
    fn start(&mut self, tag: Tag<'_>, src: LineRange, range: Range<usize>) {
        match tag {
            Tag::Paragraph => {
                self.close_implicit();
                self.mark_loose();
                self.push_inline(InlineKind::Paragraph, src);
            }
            Tag::Heading { level, .. } => {
                self.close_implicit();
                self.push_inline(InlineKind::Heading(level as u8), src);
            }
            Tag::BlockQuote(_) if self.nesting == MAX_NESTING => {
                self.push_blocks(Container::Other, src);
            }
            Tag::BlockQuote(kind) => {
                self.nesting += 1;
                // GitHub's form, alone on its line: the title is the kind's name
                let callout = kind.map(|kind| {
                    let name = gfm_name(kind);
                    Callout {
                        kind: CalloutKind::of(name, self.obsidian),
                        title: self.name_title(name, src.start),
                        fold: None,
                    }
                });
                self.push_blocks(Container::Quote(callout), src);
                if kind.is_none() {
                    self.opening =
                        marker(self.source, range.start, self.obsidian).map(Opening::Callout);
                }
            }
            Tag::CodeBlock(kind) => {
                self.close_implicit();
                let (lang, fenced) = match kind {
                    CodeBlockKind::Fenced(info) => (fence::language(&info), true),
                    CodeBlockKind::Indented => (None, false),
                };
                self.stack.push(Frame::Code {
                    lang,
                    text: String::new(),
                    src,
                    fenced,
                });
            }
            Tag::HtmlBlock => {
                self.close_implicit();
                self.stack.push(Frame::Html {
                    text: String::new(),
                    src,
                });
            }
            Tag::List(_) if self.nesting == MAX_NESTING => {
                self.push_blocks(Container::Other, src);
            }
            Tag::List(start) => {
                self.close_implicit();
                self.nesting += 1;
                self.stack.push(Frame::List {
                    start,
                    loose: false,
                    items: Vec::new(),
                    src,
                });
            }
            Tag::Item => {
                self.close_implicit();
                // an item of a list folded into its parent folds with it
                let kind = match self.stack.last() {
                    Some(Frame::List { .. }) => Container::Item,
                    _ => Container::Other,
                };
                self.push_blocks(kind, src);
                self.opening = status(self.source, range).map(Opening::Status);
            }
            Tag::FootnoteDefinition(label) => {
                let id = self.footnote_id(&label);
                self.push_blocks(Container::Footnote(id), src);
            }
            Tag::Table(aligns) => {
                self.close_implicit();
                self.stack.push(Frame::Table {
                    aligns: aligns.into_iter().map(align).collect(),
                    header: TableRow::default(),
                    rows: Vec::new(),
                    cells: Vec::new(),
                    row_start: 0,
                    row_src: src,
                    src,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(Frame::Table { row_src, .. }) = self.stack.last_mut() {
                    *row_src = src;
                }
            }
            Tag::TableCell => self.push_inline(InlineKind::Cell, src),
            Tag::Emphasis => self.style = self.style | InlineStyle::EMPH,
            Tag::Strong => self.style = self.style | InlineStyle::STRONG,
            Tag::Strikethrough => self.style = self.style | InlineStyle::STRIKE,
            // not enabled
            Tag::Superscript | Tag::Subscript => {}
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let kind = match link_type {
                    LinkType::Inline => LinkKind::Inline,
                    LinkType::Autolink => LinkKind::Autolink,
                    LinkType::Email => LinkKind::Email,
                    LinkType::WikiLink { has_pothole } => {
                        if self.dollars && dest_url.as_bytes().contains(&b'$') {
                            self.wiki_target(&range, &dest_url);
                        }
                        LinkKind::Wiki {
                            aliased: has_pothole,
                        }
                    }
                    _ => LinkKind::Reference,
                };
                self.link = Some(self.add_link(&dest_url, &title, kind, src.start));
            }
            Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            } => {
                let embed = is_embed(&dest_url, link_type);
                if matches!(link_type, LinkType::WikiLink { .. })
                    && self.dollars
                    && dest_url.as_bytes().contains(&b'$')
                {
                    self.wiki_target(&range, &dest_url);
                }
                let kind = match link_type {
                    _ if !embed => LinkKind::Image,
                    LinkType::WikiLink { has_pothole } => LinkKind::Wiki {
                        aliased: has_pothole,
                    },
                    _ => LinkKind::Inline,
                };
                let link = self.add_link(&dest_url, &title, kind, src.start);
                self.image = Some(ImageDraft {
                    link,
                    start: self.text_end(),
                    line: src.start,
                    embed,
                    aliased: matches!(link_type, LinkType::WikiLink { has_pothole: true }),
                });
            }
            // not made: its option is off (front matter is found apart, `front::find`)
            Tag::MetadataBlock(_) => {
                self.close_implicit();
                self.stack.push(Frame::Skip);
            }
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                self.push_blocks(Container::Other, src);
            }
        }
    }

    #[inline(always)]
    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Emphasis => self.style = self.style.without(InlineStyle::EMPH),
            TagEnd::Strong => self.style = self.style.without(InlineStyle::STRONG),
            TagEnd::Strikethrough => self.style = self.style.without(InlineStyle::STRIKE),
            TagEnd::Superscript | TagEnd::Subscript => {}
            TagEnd::Link => self.link = None,
            TagEnd::Image => {
                if let Some(img) = self.image.take() {
                    if !img.embed {
                        self.image_size(&img);
                    }
                    self.push_run(Run {
                        text: TextRange {
                            start: img.start,
                            end: self.text_end(),
                        },
                        style: self.style,
                        link: Some(img.link),
                        src_line: img.line,
                        kind: match img.embed {
                            true => RunKind::Embed(img.link),
                            false => RunKind::InlineImage(img.link),
                        },
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(Frame::Table {
                    header,
                    cells,
                    row_start,
                    row_src,
                    ..
                }) = self.stack.last_mut()
                {
                    let end = cells.len() as u32;
                    *header = TableRow {
                        cells: Slots {
                            start: mem::replace(row_start, end),
                            end,
                        },
                        src: *row_src,
                    };
                }
            }
            TagEnd::TableRow => {
                if let Some(Frame::Table {
                    rows,
                    cells,
                    row_start,
                    row_src,
                    ..
                }) = self.stack.last_mut()
                {
                    let end = cells.len() as u32;
                    rows.push(TableRow {
                        cells: Slots {
                            start: mem::replace(row_start, end),
                            end,
                        },
                        src: *row_src,
                    });
                }
            }
            TagEnd::TableCell => {
                if let Some(Frame::Inline {
                    kind: InlineKind::Cell,
                    ..
                }) = self.stack.last()
                    && let Some(Frame::Inline { start, .. }) = self.stack.pop()
                {
                    self.inlines_open -= 1;
                    if !matches!(self.stack.last(), Some(Frame::Table { .. })) {
                        // a cell outside a table, which pulldown-cmark doesn't make: its runs,
                        // the last ones, go
                        self.runs.truncate(start as usize);
                        return;
                    }
                    let runs = self.close_runs(start);
                    if let Some(Frame::Table { cells, .. }) = self.stack.last_mut() {
                        cells.push(Inlines {
                            runs,
                            unit: self.units,
                        });
                        self.units += 1;
                    }
                }
            }
            _ => {
                self.close_implicit();
                self.pop_block();
            }
        }
    }

    /// Obsidian's size for image `img` (OBS-008), after the last `|` in its alt text, or the whole
    /// of a wiki embed's (`![[pic.png|100]]`): taken off the alt text, which is the rest. A wiki
    /// embed left with none reads as one without the size: its target is its alt text.
    fn image_size(&mut self, img: &ImageDraft) {
        let alt = &self.text[img.start as usize..];
        let (rest, hint) = match alt.rfind('|') {
            Some(bar) => (&alt[..bar], &alt[bar + 1..]),
            None if img.aliased => ("", alt),
            None => return,
        };
        let Some(size) = ImageSize::parse(hint) else {
            return;
        };
        let rest = rest.trim_end().len();
        self.text.truncate(img.start as usize + rest);
        if let Some(link) = self.links.get_mut(img.link as usize) {
            link.size = size;
            if rest == 0 && img.aliased {
                self.text.push_str(&link.dest);
            }
        }
    }

    #[inline(always)]
    fn text(&mut self, text: &str, line: u32) {
        if self.image.is_some() {
            self.text.push_str(text);
            return;
        }
        match self.stack.last_mut() {
            Some(Frame::Code { text: code, .. } | Frame::Html { text: code, .. }) => {
                code.push_str(text);
            }
            Some(Frame::Skip) => {}
            _ => self.push_text(text, self.style, line),
        }
    }

    /// The end of [`Builder::text`], where the next run's text starts. A document of 4 GiB of
    /// inline text or more panics here, and shows as plain.
    fn text_end(&self) -> u32 {
        u32::try_from(self.text.len()).expect("inline text under 4 GiB")
    }

    /// `text`'s range once written to the end of [`Builder::text`].
    fn add_text(&mut self, text: &str) -> TextRange {
        let start = self.text_end();
        self.text.push_str(text);
        TextRange {
            start,
            end: self.text_end(),
        }
    }

    #[inline]
    fn push_text(&mut self, text: &str, style: InlineStyle, line: u32) {
        self.push_text_as(text, style, line, RunKind::Text);
    }

    /// Typeset math (OBS-003): a run of `text` in the style around it, `tex` searched behind it.
    fn push_math(&mut self, text: &str, tex: &str, line: u32) {
        if self.image.is_some() {
            self.text.push_str(text);
            return;
        }
        let id = self.math.len() as MathId;
        let tex = self.add_text(tex);
        self.math.push(tex);
        let runs = self.runs.len();
        self.push_text_as(text, self.style, line, RunKind::Math(id));
        // where no run can go, its TeX goes too
        if self.runs.len() == runs {
            self.math.pop();
            self.text.truncate(tex.start as usize);
        }
    }

    #[inline]
    fn push_text_as(&mut self, text: &str, style: InlineStyle, line: u32, kind: RunKind) {
        if self.image.is_some() {
            self.text.push_str(text);
            return;
        }
        if self.after_math {
            self.text_after_math(line);
        }
        let text = self.add_text(text);
        self.push_run(Run {
            text,
            style,
            link: self.link,
            src_line: line,
            kind,
        });
    }

    /// The rest of a paragraph split by display math starts where its text does, on `line`.
    /// Out of line, so [`Builder::push_text_as`] stays small enough to inline (PAR-010).
    #[cold]
    #[inline(never)]
    fn text_after_math(&mut self, line: u32) {
        self.after_math = false;
        if let Some(Frame::Inline { src, .. }) = self.stack.last_mut() {
            *src = LineRange::new(line, src.end.max(line));
        }
    }

    /// Whether a break on `line` goes, after display math, or ends a callout's title (the rest
    /// of the paragraph is the body's). Out of line, for [`Builder::push_break`].
    #[cold]
    #[inline(never)]
    fn break_taken(&mut self, line: u32) -> bool {
        if self.after_math {
            return true;
        }
        matches!(
            self.stack.last(),
            Some(Frame::Inline {
                kind: InlineKind::Title,
                ..
            })
        ) && self.end_title(Some(line))
    }

    fn push_break(&mut self, kind: RunKind, line: u32) {
        if self.image.is_some() {
            self.text.push(' ');
            return;
        }
        // a title is open only while its marker is read: one test of each for every break
        if (self.after_math || self.opening.is_some()) && self.break_taken(line) {
            return;
        }
        let text = self.add_text("");
        self.push_run(Run {
            text,
            style: self.style,
            link: self.link,
            src_line: line,
            kind,
        });
    }

    fn inline_html(&mut self, html: &str, line: u32) {
        let tag = html.trim().to_ascii_lowercase();
        let is_br = tag
            .strip_prefix("<br")
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric()));
        if is_br {
            self.push_break(RunKind::HardBreak, line);
        }
        // Other inline tags are dropped; the text between them renders as ordinary text.
    }

    fn footnote_ref(&mut self, label: &str, line: u32) {
        let id = self.footnote_id(label);
        self.ref_order.push(id);
        let link = self.add_link(label, "", LinkKind::Footnote(id), line);
        let text = self.add_text("");
        self.push_run(Run {
            text,
            style: self.style,
            link: Some(link),
            src_line: line,
            kind: RunKind::FootnoteRef(id),
        });
    }

    /// Push `run`, whose text is the end of [`Builder::text`]; where no run can go, its text
    /// goes too.
    fn push_run(&mut self, run: Run) {
        if !matches!(self.stack.last(), Some(Frame::Inline { .. })) {
            if !matches!(self.stack.last(), Some(Frame::Blocks { .. })) {
                debug_assert_eq!(run.text.end, self.text_end());
                self.text.truncate(run.text.start as usize);
                return;
            }
            self.push_inline(InlineKind::Implicit, LineRange::line(run.src_line));
        }
        if let Some(Frame::Inline { .. }) = self.stack.last() {
            self.runs.push(run);
        }
    }

    fn push_inline(&mut self, kind: InlineKind, src: LineRange) {
        self.after_math = false;
        self.inlines_open += 1;
        self.stack.push(Frame::Inline {
            kind,
            start: self.runs.len() as u32,
            src,
        });
    }

    /// The runs of the inline frame just popped, which began at `start`: [`Builder::runs`] from
    /// there on, unless another inline frame is open below it (pulldown-cmark doesn't nest them,
    /// but nothing here relies on it). Then its runs move ahead of the open frames', which stay
    /// end to end after them, so each frame's runs remain one range.
    fn close_runs(&mut self, start: u32) -> Slots {
        // the marks on the frame's lines, by every mark's, in debug builds (the fuzzers', `cargo
        // test`): none where `may_mark` lets the frame pass
        debug_assert!(self.may_mark(start) || !self.any_mark_on(start));
        // what most frames skip, out of line, so this stays small: inlined, the mark search
        // saved and restored a dozen registers a frame (PAR-022)
        if self.may_mark(start) || self.obsidian {
            self.marks_and_tags(start as usize);
        }
        debug_assert_eq!(
            self.inlines_open as usize,
            (self.stack.iter())
                .filter(|f| matches!(f, Frame::Inline { .. }))
                .count()
        );
        if self.inlines_open == 0 {
            let end = self.runs.len() as u32;
            return Slots { start, end };
        }
        self.nested_runs(start)
    }

    /// A closing frame's highlights, near a mark, and its tags, in Obsidian's dialect.
    #[inline(never)]
    fn marks_and_tags(&mut self, start: usize) {
        if self.may_mark(start as u32) && self.marked(start) {
            self.highlight(start);
        }
        if self.obsidian {
            self.tags(start);
        }
    }

    /// [`Builder::close_runs`] of a frame inside another inline frame: its runs, from `start`,
    /// moved ahead of the open frames'.
    #[inline(never)]
    fn nested_runs(&mut self, start: u32) -> Slots {
        let end = self.runs.len() as u32;
        let n = end - start;
        let mut first = start;
        for f in &mut self.stack {
            if let Frame::Inline { start: open, .. } = f {
                first = first.min(*open);
                *open += n;
            }
        }
        if first < start {
            self.runs[first as usize..].rotate_right(n as usize);
        }
        Slots {
            start: first,
            end: first + n,
        }
    }

    /// Whether a mark may be on the lines of the inline frame closing, whose runs start at
    /// `start`: not when they're between [`Builder::mark_lines`].
    #[inline(always)]
    fn may_mark(&self, start: u32) -> bool {
        let (Some(first), Some(last)) = (self.runs.get(start as usize), self.runs.last()) else {
            return false;
        };
        let (before, next) = self.mark_lines;
        before >= first.src_line.min(last.src_line) || next <= first.src_line.max(last.src_line)
    }

    /// Whether any of [`Builder::marks`] is on the lines of the runs from `start`, each mark's
    /// line searched: [`Builder::may_mark`]'s check, in debug builds.
    fn any_mark_on(&self, start: u32) -> bool {
        let runs = &self.runs[start as usize..];
        let (Some(first), Some(last)) = (runs.first(), runs.last()) else {
            return false;
        };
        let lines = first.src_line.min(last.src_line)..=first.src_line.max(last.src_line);
        (self.marks.iter())
            .any(|&q| lines.contains(&(self.line_starts.partition_point(|&s| s <= q) as u32)))
    }

    /// [`Builder::mark_lines`] for the cursor at `i`.
    fn lines_beside(&self, i: usize) -> (u32, u32) {
        let line = |q: u32| self.line_starts.partition_point(|&s| s <= q) as u32;
        (
            (i.checked_sub(1)).map_or(0, |p| line(self.marks[p])),
            self.marks.get(i).map_or(u32::MAX, |&q| line(q)),
        )
    }

    /// Whether a run of `=`s is on the lines of the inline frame closing, whose runs start at
    /// `start`: only then may it hold a highlight. Found from [`Builder::mark`] on, as frames
    /// close in source order, nearly (a binary search a frame cost a note with one `==` 1–2% of
    /// its parse). Out of line: [`Builder::may_mark`] lets only the frames near a mark through.
    #[inline(never)]
    fn marked(&mut self, start: usize) -> bool {
        let runs = &self.runs[start..];
        let (Some(first), Some(last)) = (runs.first(), runs.last()) else {
            return false;
        };
        let (a, b) = (
            first.src_line.min(last.src_line),
            first.src_line.max(last.src_line),
        );
        let from = (a.checked_sub(1)).and_then(|a| self.line_starts.get(a as usize).copied());
        let from = from.unwrap_or(0);
        let to = self
            .line_starts
            .get(b as usize)
            .copied()
            .unwrap_or(u32::MAX);
        let marks = &self.marks;
        let mut i = self.mark;
        if i > 0 && marks[i - 1] >= from {
            // a frame before the last one's
            i = marks.partition_point(|&q| q < from);
        }
        while marks.get(i).is_some_and(|&q| q < from) {
            i += 1;
        }
        let found = marks.get(i).is_some_and(|&q| q < to);
        if self.mark != i {
            self.mark = i;
            self.mark_lines = self.lines_beside(i);
        }
        found
    }

    /// Obsidian's highlights (OBS-005), in every dialect (ADR-45: `==` means nothing else), in
    /// the runs of the inline frame closing, `runs[start..]`, the last ones: each `==` that pairs
    /// is taken out of its run, and the runs between a pair are highlighted, across emphasis,
    /// links and breaks. A `==` pairs as GFM's `~~` does: two `=` exactly, outside code, opening
    /// where it's left-flanking and closing where it's right-flanking, with the nearest open one
    /// before it (so `a == b` and `x==y` alone stay as written). Its neighbors are the text as
    /// shown, with a break as a space and code, an image or a footnote reference as punctuation.
    /// The color emoji after a `==` (`==🔴…==`) stays, as text.
    /// Out of line, so `close_runs` stays small: most frames have no `==` on their lines.
    #[inline(never)]
    fn highlight(&mut self, start: usize) {
        let runs = &self.runs[start..];
        let text = &self.text[..];
        let mut delims: Vec<Delim> = Vec::new();
        for (i, run) in runs.iter().enumerate() {
            if run.kind != RunKind::Text || run.style.contains(InlineStyle::CODE) {
                continue;
            }
            let s = run.text.of(text);
            let mut at = 0;
            while let Some(found) = memchr::memchr(b'=', &s.as_bytes()[at..]) {
                let p = at + found;
                at = p + s[p..].bytes().take_while(|&c| c == b'=').count();
                if at - p != 2 {
                    continue;
                }
                let before = (s[..p].chars().next_back())
                    .unwrap_or_else(|| edge(i.checked_sub(1).map(|i| &runs[i]), text, true));
                let after =
                    (s[at..].chars().next()).unwrap_or_else(|| edge(runs.get(i + 1), text, false));
                let (open, close) = flanking(before, after);
                if open || close {
                    delims.push(Delim {
                        run: i,
                        at: run.text.start + p as u32,
                        open,
                        close,
                        side: None,
                    });
                }
            }
        }
        let mut openers: Vec<usize> = Vec::new();
        let mut paired = false;
        for d in 0..delims.len() {
            if delims[d].close
                && let Some(o) = openers.pop()
            {
                delims[o].side = Some(Side::Open);
                delims[d].side = Some(Side::Close);
                paired = true;
            } else if delims[d].open {
                openers.push(d);
            }
        }
        if !paired {
            return;
        }
        let old = self.runs.split_off(start);
        let mut depth = 0u32;
        let mut delims = delims
            .iter()
            .filter_map(|d| Some((d.run, d.at, d.side?)))
            .peekable();
        for (i, run) in old.into_iter().enumerate() {
            let mut from = run.text.start;
            let mut split = false;
            while let Some((_, at, side)) = delims.next_if(|d| d.0 == i) {
                self.push_marked(run, from..at, depth);
                depth = match side {
                    Side::Open => depth + 1,
                    Side::Close => depth - 1,
                };
                from = at + 2;
                split = true;
            }
            if split {
                self.push_marked(run, from..run.text.end, depth);
            } else {
                let mut run = run;
                if depth > 0 {
                    run.style = run.style | InlineStyle::HIGHLIGHT;
                }
                self.runs.push(run);
            }
        }
    }

    /// Obsidian's tags (OBS-006), in its dialect only (ADR-45: a `#include` or a hashtag in a
    /// README isn't one), in the runs of the inline frame closing, `runs[start..]`: a `#` at the
    /// frame's start or after a space or a break, then [`is_tag_char`]s, at least one of them not
    /// a digit (`#12` is no tag, nor `C#`). Not in code or a link (a URL's `#` follows no space
    /// anyway), nor escaped (`\#include`), nor the keycap emoji `#️⃣`. Its text is styled `TAG`,
    /// the `#` too, across text runs of one style that pulldown-cmark split (`#my_tag` comes as
    /// three). Out of line, as [`Builder::highlight`].
    #[inline(never)]
    fn tags(&mut self, start: usize) {
        let taggable = |r: &Run| {
            r.kind == RunKind::Text && r.link.is_none() && !r.style.contains(InlineStyle::CODE)
        };
        let runs = &self.runs[start..];
        let text = &self.text[..];
        if !runs
            .iter()
            .any(|r| taggable(r) && memchr::memchr(b'#', r.text.of(text).as_bytes()).is_some())
        {
            return;
        }
        // each stretch of text runs of one style, end to end in the text, scanned whole
        let mut tags: Vec<Range<u32>> = Vec::new();
        let mut i = 0;
        while i < runs.len() {
            if !taggable(&runs[i]) {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while (runs.get(j)).is_some_and(|r| {
                taggable(r) && r.style == runs[i].style && r.text.start == runs[j - 1].text.end
            }) {
                j += 1;
            }
            let (from, to) = (runs[i].text.start, runs[j - 1].text.end);
            let s = &text[from as usize..to as usize];
            let mut before = match i.checked_sub(1).map(|p| &runs[p]) {
                None => ' ',
                Some(r) if is_break(r) => ' ',
                Some(r) => r.text.of(text).chars().next_back().unwrap_or('.'),
            };
            let mut chars = s.char_indices().peekable();
            while let Some((at, c)) = chars.next() {
                let escaped = || self.escaped.binary_search(&(from + at as u32)).is_ok();
                // `#️⃣` is the keycap emoji, no tag
                let keycap = matches!(chars.peek(), Some((_, '\u{FE0F}' | '\u{20E3}')));
                if c == '#' && before.is_whitespace() && !keycap && !escaped() {
                    let mut end = at + 1;
                    let mut named = false;
                    while let Some(&(p, c)) = chars.peek().filter(|&&(_, c)| is_tag_char(c)) {
                        named |= !c.is_ascii_digit();
                        end = p + c.len_utf8();
                        chars.next();
                    }
                    if named {
                        tags.push(from + at as u32..from + end as u32);
                    }
                    before = s[..end].chars().next_back().unwrap_or(c);
                    continue;
                }
                before = c;
            }
            i = j;
        }
        if tags.is_empty() {
            return;
        }
        let old = self.runs.split_off(start);
        let mut tags = tags.into_iter().peekable();
        for run in old {
            if !taggable(&run) {
                self.runs.push(run);
                continue;
            }
            let mut at = run.text.start;
            while let Some(tag) = tags.peek().filter(|t| t.start < run.text.end).cloned() {
                let tag = tag.start.max(at)..tag.end.min(run.text.end);
                self.push_styled(run, at..tag.start, InlineStyle::NONE);
                self.push_styled(run, tag.clone(), InlineStyle::TAG);
                at = tag.end;
                if tags.peek().is_some_and(|t| t.end > run.text.end) {
                    break;
                }
                tags.next();
            }
            self.push_styled(run, at..run.text.end, InlineStyle::NONE);
        }
    }

    /// The part `range` of text run `run`, with `style` added to its own; none if empty.
    fn push_styled(&mut self, run: Run, range: Range<u32>, style: InlineStyle) {
        if !range.is_empty() {
            self.runs.push(Run {
                text: TextRange {
                    start: range.start,
                    end: range.end,
                },
                style: run.style | style,
                ..run
            });
        }
    }

    /// The part `range` of text run `run`, highlighted inside `depth` highlights; none if empty.
    fn push_marked(&mut self, run: Run, range: Range<u32>, depth: u32) {
        if range.is_empty() {
            return;
        }
        let style = match depth {
            0 => run.style,
            _ => run.style | InlineStyle::HIGHLIGHT,
        };
        self.runs.push(Run {
            text: TextRange {
                start: range.start,
                end: range.end,
            },
            style,
            ..run
        });
    }

    fn push_blocks(&mut self, kind: Container, src: LineRange) {
        self.close_implicit();
        self.stack.push(Frame::Blocks {
            kind,
            blocks: Vec::new(),
            src,
            task: None,
        });
    }

    /// Forced inline: a test, and a call only for a tight item's text. Its callers grew with
    /// `start`'s two copies, and it went out of line, a call at every block (PAR-010).
    #[inline(always)]
    fn close_implicit(&mut self) {
        if let Some(Frame::Inline {
            kind: InlineKind::Implicit,
            ..
        }) = self.stack.last()
        {
            self.pop_block();
        }
    }

    /// A `Paragraph` directly inside a list item makes the list loose.
    fn mark_loose(&mut self) {
        let n = self.stack.len();
        if n >= 2
            && let Frame::Blocks {
                kind: Container::Item,
                ..
            } = &self.stack[n - 1]
            && let Frame::List { loose, .. } = &mut self.stack[n - 2]
        {
            *loose = true;
        }
    }

    fn add_link(&mut self, dest: &str, title: &str, kind: LinkKind, line: u32) -> LinkId {
        if let Some(m) = self.mask {
            return self.add_masked_link(dest, title, kind, line, m);
        }
        self.links.push(LinkRef {
            dest: dest.into(),
            title: title.into(),
            kind,
            src_line: line,
            size: ImageSize::NONE,
        });
        self.links.len() as LinkId - 1
    }

    /// [`Builder::add_link`] in a pass with masked `$`s, its destination and title given theirs
    /// back: a reference link's (or image's) are its definition's, out of its range, where
    /// [`Builder::rare_from`] doesn't find them. A `$` masked before the first pass may stand in a
    /// title on the lines after a definition's (`[a]: /u` then `($b … $7)`), as [`line_safe`]
    /// reads only a definition's line and the next, and a label's copy in one ([`label_copies`]).
    /// Out of line, off the parse's loop: any other link's were given back with their event.
    #[cold]
    #[inline(never)]
    fn add_masked_link(
        &mut self,
        dest: &str,
        title: &str,
        kind: LinkKind,
        line: u32,
        m: u8,
    ) -> LinkId {
        let dest = unmasked(CowStr::Borrowed(dest), m);
        let title = unmasked(CowStr::Borrowed(title), m);
        self.links.push(LinkRef {
            dest: (*dest).into(),
            title: (*title).into(),
            kind,
            src_line: line,
            size: ImageSize::NONE,
        });
        self.links.len() as LinkId - 1
    }

    /// The footnote of `label`, matched as pulldown-cmark matches labels ([`label_key`]):
    /// `[^A]` is `[^a]`'s.
    fn footnote_id(&mut self, label: &str) -> FootnoteId {
        // most labels are their key already: lowercase ASCII without blanks
        let folded =
            (label.bytes()).all(|c| c.is_ascii() && !c.is_ascii_uppercase() && !blank_byte(c));
        let key = match folded {
            true => Cow::Borrowed(label),
            false => Cow::Owned(label_key(label)),
        };
        if let Some(&id) = self.footnote_ids.get(&*key) {
            return id;
        }
        let id = self.footnotes.len() as FootnoteId;
        self.footnotes.push(Footnote {
            label: label.into(),
            number: 0,
            anchor: None,
            blocks: Vec::new(),
            src: LineRange::NONE,
        });
        self.footnote_ids.insert(key.into(), id);
        id
    }

    fn pop_block(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        // a title is open only while its marker is read: a test of that for every block
        if self.opening.is_some()
            && let Some(Frame::Inline {
                kind: InlineKind::Title,
                ..
            }) = self.stack.last()
            && self.end_title(None)
        {
            // a paragraph of one line: the title alone
            return;
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if let Frame::Inline { .. } = frame {
            self.inlines_open -= 1;
        }
        let block = match frame {
            Frame::Inline { kind, start, src } => match kind {
                InlineKind::Paragraph | InlineKind::Implicit | InlineKind::Cell
                    if self.runs.len() == start as usize =>
                {
                    return;
                }
                InlineKind::Heading(level) => {
                    let inlines = Inlines {
                        runs: self.close_runs(start),
                        unit: self.next_unit(),
                    };
                    let text = plain_text(inlines.runs.of(&self.runs), &self.text);
                    let slug = self.slugs.slug(&text);
                    self.anchors.push(Anchor {
                        slug: slug.into(),
                        text: text.into(),
                        kind: AnchorKind::Heading(level),
                    });
                    Block {
                        kind: BlockKind::Heading {
                            level,
                            inlines,
                            anchor: self.anchors.len() as u32 - 1,
                        },
                        src,
                    }
                }
                // one ending in a block id (OBS-009) apart, so the others take the way they took
                // before it: returned through `paragraph_block`, every paragraph cost ~75
                // instructions more (the lists input's parse +1.1%)
                InlineKind::Implicit | InlineKind::Paragraph => match self.block_id(start) {
                    Some(id) => {
                        let implicit = kind == InlineKind::Implicit;
                        match self.paragraph_block(start, src, implicit, id) {
                            Some(block) => block,
                            None => return,
                        }
                    }
                    None => {
                        let src = match kind {
                            InlineKind::Implicit => self.implicit_lines(start, src),
                            _ => src,
                        };
                        // looked for only where embeds are transcluded: its call, and the block
                        // it gave back, cost every paragraph (PAR-010)
                        if self.embedding.is_some()
                            && let Some(block) = self.embed(start, src)
                        {
                            block
                        } else {
                            // the frame under a paragraph's is the container it goes in, its
                            // block made in place there: through `push_block`'s look down the
                            // stack, a paragraph's close cost ~145 instructions more than
                            // v1.1.0's, its block moved between calls (PAR-010)
                            let block = self.paragraph(start, src);
                            match self.stack.last_mut() {
                                Some(Frame::Blocks { blocks, .. }) => blocks.push(block),
                                _ => self.push_block(block),
                            }
                            return;
                        }
                    }
                },
                InlineKind::Cell | InlineKind::Title => self.paragraph(start, src),
            },
            Frame::Blocks {
                kind,
                blocks,
                src,
                task,
            } => match kind {
                Container::Item => {
                    if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                        items.push(ListItem { task, blocks, src });
                    }
                    return;
                }
                Container::Footnote(id) => {
                    let label = self.footnotes[id as usize].label.clone();
                    let slug = self.slugs.slug(&format!("fn-{label}"));
                    self.anchors.push(Anchor {
                        slug: slug.into(),
                        text: label,
                        kind: AnchorKind::Footnote(id),
                    });
                    let note = &mut self.footnotes[id as usize];
                    note.anchor = Some(self.anchors.len() as u32 - 1);
                    note.blocks = blocks;
                    note.src = src;
                    return;
                }
                Container::Quote(callout) => {
                    self.nesting -= 1;
                    Block {
                        kind: BlockKind::Quote { callout, blocks },
                        src,
                    }
                }
                Container::Other => {
                    for b in blocks {
                        self.push_block(b);
                    }
                    return;
                }
                Container::Root => return,
            },
            Frame::List {
                start,
                loose,
                items,
                src,
            } => {
                self.nesting -= 1;
                Block {
                    kind: BlockKind::List(List {
                        start,
                        tight: !loose,
                        items,
                    }),
                    src,
                }
            }
            Frame::Code {
                lang,
                text,
                src,
                fenced,
            } => self.code_block(lang, text, src, src.start + u32::from(fenced), fenced),
            Frame::Html { text, src } => {
                let trimmed = text.trim();
                if trimmed.is_empty() || (trimmed.starts_with("<!--") && trimmed.ends_with("-->")) {
                    return;
                }
                Block {
                    kind: BlockKind::Html(trimmed.into()),
                    src,
                }
            }
            Frame::Table {
                aligns,
                header,
                rows,
                mut cells,
                src,
                ..
            } => {
                self.tables += 1;
                cells.shrink_to_fit();
                Block {
                    kind: BlockKind::Table(Table {
                        id: self.tables - 1,
                        aligns,
                        header,
                        rows,
                        cells,
                    }),
                    src,
                }
            }
            Frame::Skip => return,
        };
        self.push_block(block);
    }

    /// A code block of `text` in `lang`, spanning `src`, whose first line is source line
    /// `first_line`.
    fn code_block(
        &mut self,
        lang: Option<Box<str>>,
        text: String,
        src: LineRange,
        first_line: u32,
        fenced: bool,
    ) -> Block {
        Block {
            kind: BlockKind::Code(self.code(lang, text, first_line, fenced)),
            src,
        }
    }

    /// [`Builder::code_block`]'s code, numbered and given its search units.
    fn code(
        &mut self,
        lang: Option<Box<str>>,
        text: String,
        first_line: u32,
        fenced: bool,
    ) -> CodeBlock {
        self.code_blocks += 1;
        let first_unit = self.units;
        self.units += code_line_count(&text) as UnitId;
        let diagram = super::diagram(lang.as_deref(), &text);
        CodeBlock {
            id: self.code_blocks - 1,
            lang,
            hash: super::content_hash(&text),
            text: text.into(),
            first_line,
            fenced,
            whole_file: false,
            first_unit,
            diagram,
            colors: None,
        }
    }

    /// The note's front matter, `front`, read into its properties (ADR-46, OBS-010): the first
    /// block, made before any other, so its source and rows take the first code id and search
    /// units. Each row's value is runs of plain text, but for its `[[links]]` and URLs, which are
    /// links, and a `tags` value's items, tags (`#` before each, as in the note). Not for an
    /// Excalidraw drawing's, whose keys are the plugin's: [`Builder::drawing`] names it. The
    /// front matter is found again here, out of line, so that [`markdown`] keeps only its end. Its
    /// lines are found by a search of their own, not [`Builder::line_of`]: called here too, that
    /// was no longer inlined into [`Builder::event`], and every note's parse took 1.4% more
    /// instructions, front matter or not.
    #[inline(never)]
    fn properties(&mut self) {
        let Some(front) = super::front::read(self.source, self.obsidian) else {
            return;
        };
        let body = &self.source[front.body.clone()];
        let line = |at: usize| self.line_starts.partition_point(|&s| s as usize <= at) as u32;
        let first = line(front.body.start);
        let close = line(front.end.saturating_sub(1)).max(first);
        let read = props::read(body, front.lang);
        let lang = Some(front.lang.name().into());
        let source = self.code(lang, body.to_owned(), first, true);
        let mut rows = Vec::new();
        let mut title = None;
        for p in read.iter().flatten().filter(|p| !props::is_hidden(&p.key)) {
            let src = LineRange::new(first + p.lines.start, first + p.lines.end - 1);
            if &*p.key == "title"
                && let props::Value::Text(t) = &p.value
                && !t.trim().is_empty()
            {
                title = Some(t.trim().into());
            }
            let start = self.runs.len() as u32;
            self.value(&p.value, props::is_tags(&p.key), src.start);
            let runs = Slots {
                start,
                end: self.runs.len() as u32,
            };
            let unit = self.next_unit();
            rows.push(Property {
                key: (*p.key).into(),
                value: Inlines { runs, unit },
                src,
            });
        }
        self.push_block(Block {
            kind: BlockKind::Properties(Box::new(Properties {
                rows,
                source,
                read: read.is_some(),
                title,
            })),
            src: LineRange::new(1, close),
        });
    }

    /// An Excalidraw drawing's notice (OBS-015), a callout of one line in place of its front
    /// matter and the plugin's banner, which end at `end`: the drawing's text follows it, its data
    /// cut off ([`super::front::drawing`]).
    #[cold]
    fn drawing(&mut self, end: usize) {
        let close = self.line_starts.partition_point(|&s| (s as usize) < end) as u32;
        let start = self.runs.len() as u32;
        self.value_run(DRAWING, InlineStyle::NONE, None, 1);
        let title = Inlines {
            runs: self.close_runs(start),
            unit: self.next_unit(),
        };
        self.push_block(Block {
            kind: BlockKind::Quote {
                callout: Some(Callout {
                    kind: CalloutKind::Info,
                    title,
                    fold: None,
                }),
                blocks: Vec::new(),
            },
            src: LineRange::new(1, close.max(1)),
        });
    }

    /// A property's value as runs on line `line`: a list's items joined by `, `, tags' by a
    /// space.
    fn value(&mut self, value: &props::Value<'_>, tags: bool, line: u32) {
        if tags {
            for (i, tag) in props::items(value, true).into_iter().enumerate() {
                if i > 0 {
                    self.value_run(" ", InlineStyle::NONE, None, line);
                }
                let tag = tag.strip_prefix('#').unwrap_or(tag);
                self.value_run("#", InlineStyle::TAG, None, line);
                self.value_run(tag, InlineStyle::TAG, None, line);
            }
            return;
        }
        match value {
            props::Value::Empty => {}
            props::Value::Text(t) => self.value_text(t, line),
            props::Value::List(items) => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.value_run(", ", InlineStyle::NONE, None, line);
                    }
                    self.value_text(item, line);
                }
            }
            props::Value::Cut(t) => {
                self.value_text(t, line);
                self.value_run(" …", InlineStyle::NONE, None, line);
            }
        }
    }

    /// A value's `text`: `[[links]]` (with an alias after a `|`) and `http(s)://` URLs are links,
    /// the rest plain, as Obsidian shows a text property.
    fn value_text(&mut self, text: &str, line: u32) {
        // where the next `[[` and the next URL start, each looked for again only once passed, and
        // whether a `]]` may still close one: looked for from the rest of the value each time,
        // a value of many links, or of many `[[` that don't close, was quadratic (PAR-013)
        let find = |from: usize, p: &str| text[from..].find(p).map(|i| from + i);
        let url_at = |from| {
            ["https://", "http://"]
                .iter()
                .filter_map(|p| find(from, p))
                .min()
        };
        let (mut wiki, mut url) = (find(0, "[["), url_at(0));
        let mut closes = true;
        let mut at = 0;
        while at < text.len() {
            if wiki.is_some_and(|w| w < at) {
                wiki = find(at, "[[");
            }
            if url.is_some_and(|u| u < at) {
                url = url_at(at);
            }
            let next = wiki.into_iter().chain(url).min().unwrap_or(text.len());
            self.value_run(&text[at..next], InlineStyle::NONE, None, line);
            at = next;
            let rest = &text[at..];
            if rest.is_empty() {
                break;
            }
            let end = match rest.strip_prefix("[[") {
                Some(inner) if closes => inner.find("]]").or_else(|| {
                    closes = false;
                    None
                }),
                _ => None,
            };
            if let Some(end) = end.filter(|&end| end > 0) {
                let inner = &rest[2..];
                let (dest, shown) = inner[..end].split_once('|').unwrap_or((&inner[..end], ""));
                let kind = LinkKind::Wiki {
                    aliased: !shown.is_empty(),
                };
                let link = self.add_link(dest.trim(), "", kind, line);
                let shown = if shown.is_empty() { dest } else { shown };
                self.value_run(shown.trim(), InlineStyle::NONE, Some(link), line);
                at += 2 + end + 2;
            } else if rest.starts_with("http") {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                // punctuation that ends a sentence ends the URL
                let end = rest[..end]
                    .trim_end_matches(['.', ',', ';', ':', ')', ']', '"', '\''])
                    .len();
                let url = &rest[..end];
                let link = self.add_link(url, "", LinkKind::Autolink, line);
                self.value_run(url, InlineStyle::NONE, Some(link), line);
                at += end;
            } else {
                // `[[` that doesn't close: as written
                self.value_run("[[", InlineStyle::NONE, None, line);
                at += 2;
            }
        }
    }

    /// A run of a property's value: `text` in `style`, a link's when `link` is.
    fn value_run(&mut self, text: &str, style: InlineStyle, link: Option<LinkId>, line: u32) {
        if text.is_empty() {
            return;
        }
        let text = self.add_text(text);
        self.runs.push(Run {
            text,
            style,
            link,
            src_line: line,
            kind: RunKind::Text,
        });
    }

    /// `$$…$$` (OBS-003): typeset as Unicode, a block of its rows set centered; else the TeX as
    /// written, a block of its own lines labeled `math`, as a ```` ```math ```` fence shows. Its
    /// paragraph splits around it, the text before and after paragraphs of their own. Where a
    /// block can't go (a heading, a table cell, a link's text), it is inline: typeset when it
    /// is one row, else the TeX as code on one line.
    fn display_math(&mut self, tex: &str, src: LineRange) {
        let rows = tex::display(tex);
        let open = match self.stack.last_mut() {
            Some(Frame::Inline {
                kind: kind @ (InlineKind::Paragraph | InlineKind::Implicit),
                start,
                src: para,
            }) if self.image.is_none() && self.link.is_none() => {
                let end = para.end.max(src.end);
                // the paragraph so far ends on the line its last run is on
                let before = &self.runs[*start as usize..];
                if let Some(last) = before.iter().rev().find(|r| !is_break(r)) {
                    para.end = last.src_line.max(para.start);
                }
                Some((*kind, *start, end))
            }
            _ => None,
        };
        let Some((kind, start, end)) = open else {
            if let Some([row]) = rows.as_deref() {
                return self.push_math(&row.text, tex, src.start);
            }
            let flat = tex.trim().replace('\n', " ");
            return self.push_text(&flat, self.style | InlineStyle::CODE, src.start);
        };
        // the breaks before the math end no line of the paragraph
        while self.runs.len() > start as usize && self.runs.last().is_some_and(is_break) {
            self.runs.pop();
        }
        self.pop_block();
        let block = match rows {
            // its runs go end to end, so no other inline frame may be open
            Some(rows) if self.inlines_open == 0 => self.math_block(tex, &rows, src),
            _ => {
                let (body, first_line) = match tex.strip_prefix('\n') {
                    Some(rest) => (rest, src.start + 1),
                    None => (tex, src.start),
                };
                let body = body.strip_suffix('\n').unwrap_or(body);
                self.code_block(Some("math".into()), body.into(), src, first_line, false)
            }
        };
        self.push_block(block);
        self.push_inline(kind, LineRange::new(src.end, end));
        self.after_math = true;
    }

    /// Display math's typeset `rows` as a block: a run each, hard breaks between them, each on
    /// the source line its TeX starts on. The rows of one line (a matrix's) share its TeX.
    fn math_block(&mut self, tex: &str, rows: &[tex::Row], src: LineRange) -> Block {
        let start = self.runs.len() as u32;
        let mut last: Option<&std::ops::Range<usize>> = None;
        // the lines before the last row's TeX, counted on from there: rows come in source order,
        // and counting from the block's start each row was quadratic in a tall block (PAR-013)
        let (mut counted, mut lines) = (0, 0);
        for row in rows {
            let at = row.tex.start.min(tex.len());
            if at < counted || !tex.is_char_boundary(at) {
                (counted, lines) = (0, 0);
            }
            if tex.is_char_boundary(at) {
                lines += tex[counted..at].matches('\n').count();
                counted = at;
            } else {
                lines = tex.matches('\n').count();
            }
            let line = src.start + lines as u32;
            if self.runs.len() > start as usize {
                let text = self.add_text("");
                self.runs.push(Run {
                    text,
                    style: InlineStyle::NONE,
                    link: None,
                    src_line: line,
                    kind: RunKind::HardBreak,
                });
            }
            if last != Some(&row.tex) {
                let range = self.add_text(tex.get(row.tex.clone()).unwrap_or(tex));
                self.math.push(range);
                last = Some(&row.tex);
            }
            let id = self.math.len() as MathId - 1;
            let text = self.add_text(&row.text);
            self.runs.push(Run {
                text,
                style: InlineStyle::NONE,
                link: None,
                src_line: line,
                kind: RunKind::Math(id),
            });
        }
        let runs = Slots {
            start,
            end: self.runs.len() as u32,
        };
        let unit = self.next_unit();
        Block {
            kind: BlockKind::Math(Inlines { runs, unit }),
            src,
        }
    }

    /// Close the open title (OBS-002), the callout's once its marker was read: its runs, or the
    /// kind's name when it has none. A break on `line` ends it, and the rest of its paragraph
    /// opens as one of its own; else the paragraph ended with it. `false` when the marker wasn't
    /// read whole: the title is a plain paragraph again, left open.
    fn end_title(&mut self, line: Option<u32>) -> bool {
        let Some(Frame::Inline {
            kind: InlineKind::Title,
            start,
            src,
        }) = self.stack.pop()
        else {
            return false;
        };
        self.inlines_open -= 1;
        let marker = match self.opening.take() {
            Some(Opening::Callout(m)) => Some(m),
            other => {
                self.opening = other;
                None
            }
        };
        let callout = match marker {
            Some(m) if m.state == MarkerState::Title => {
                let blank = (self.runs[start as usize..].iter()).all(|r| is_blank(r, &self.text));
                let title = if blank {
                    self.runs.truncate(start as usize);
                    let name = &self.source[m.name];
                    self.name_title(name, src.start)
                } else {
                    Inlines {
                        runs: self.close_runs(start),
                        unit: self.next_unit(),
                    }
                };
                Some(Callout {
                    kind: m.kind,
                    title,
                    fold: m.fold,
                })
            }
            // the marker wasn't there after all (the paragraph opened elsewhere, or with no text)
            _ => None,
        };
        let Some(callout) = callout else {
            self.stack.push(Frame::Inline {
                kind: InlineKind::Paragraph,
                start,
                src,
            });
            self.inlines_open += 1;
            return false;
        };
        if let Some(Frame::Blocks {
            kind: Container::Quote(slot),
            ..
        }) = self.stack.last_mut()
        {
            *slot = Some(callout);
        }
        if let Some(line) = line {
            let rest = LineRange::new(line + 1, src.end.max(line + 1));
            self.push_inline(InlineKind::Paragraph, rest);
        }
        true
    }

    /// A callout's title that is its kind's `name` in title case, as Obsidian shows it.
    fn name_title(&mut self, name: &str, line: u32) -> Inlines {
        let start = self.runs.len() as u32;
        let text = self.text_end();
        if name.is_ascii() {
            // cased in place: char by char, a GitHub alert's title was ~320 instructions
            self.text.push_str(name);
            let title = &mut self.text[text as usize..];
            title.make_ascii_lowercase();
            let first = 1.min(title.len());
            title[..first].make_ascii_uppercase();
        } else {
            let mut chars = name.chars();
            if let Some(first) = chars.next() {
                self.text.extend(first.to_uppercase());
                self.text.extend(chars.flat_map(char::to_lowercase));
            }
        }
        self.runs.push(Run {
            text: TextRange {
                start: text,
                end: self.text_end(),
            },
            style: InlineStyle::NONE,
            link: None,
            src_line: line,
            kind: RunKind::Text,
        });
        Inlines {
            runs: self.close_runs(start),
            unit: self.next_unit(),
        }
    }

    /// `event`, with a callout's marker read out of it (OBS-002): `None` when it was all marker.
    /// The quote's first event must open a paragraph where the marker is, and the paragraph's
    /// text must hold the marker; else it is a plain quote, its paragraph a plain paragraph.
    fn read_marker<'e>(
        &mut self,
        event: Event<'e>,
        range: &Range<usize>,
        src: LineRange,
    ) -> Option<Event<'e>> {
        let Some(Opening::Callout(m)) = &mut self.opening else {
            return Some(event);
        };
        match m.state {
            MarkerState::Armed => {
                if !(matches!(event, Event::Start(Tag::Paragraph)) && range.start == m.at) {
                    self.opening = None;
                    return Some(event);
                }
                m.state = MarkerState::Reading { upto: m.at };
                self.push_inline(InlineKind::Title, src);
                None
            }
            MarkerState::Reading { upto } => match event {
                // the marker's text: ASCII without escapes or entities, so offsets are the text's
                Event::Text(t) if range.start < m.title && range.start >= upto => {
                    let upto = range.end.max(upto);
                    if upto < m.end {
                        m.state = MarkerState::Reading { upto };
                        return None;
                    }
                    m.state = MarkerState::Title;
                    title_text(&t, range.start, m.title)
                }
                _ if upto >= m.end => {
                    m.state = MarkerState::Title;
                    Some(event)
                }
                _ => {
                    // not the marker: a plain paragraph
                    self.opening = None;
                    if let Some(Frame::Inline { kind, .. }) = self.stack.last_mut() {
                        *kind = InlineKind::Paragraph;
                    }
                    Some(event)
                }
            },
            // pulldown-cmark splits the marker's text at `[` and `]`: the spaces before the title
            // can come after it was read
            MarkerState::Title => match event {
                Event::Text(t) if range.start < m.title => title_text(&t, range.start, m.title),
                _ => Some(event),
            },
        }
    }

    /// The block of a paragraph's inline frame just popped (a tight item's text, `implicit`),
    /// which began at `start` and ends in the block id `id` (OBS-009): the note it embeds, else
    /// the paragraph. The id names it, or the item it starts; an id alone names the block before
    /// it. In Obsidian's dialect the id is hidden, and a paragraph of the id alone goes (`None`);
    /// in others it shows, but names the block all the same, so a link to it finds it.
    #[cold]
    fn paragraph_block(
        &mut self,
        start: u32,
        src: LineRange,
        implicit: bool,
        id: BlockId,
    ) -> Option<Block> {
        let mut id = Some(id);
        // the id alone names the block before it; with none, it's text
        let before = match &id {
            Some(i) if i.keep == 0 => {
                let named = self
                    .last_block()
                    .and_then(|b| Some((first_unit(b)?, b.src)));
                if named.is_none() {
                    id = None;
                }
                named
            }
            _ => None,
        };
        let hidden = id.as_ref().filter(|_| self.obsidian);
        if let Some(i) = hidden {
            self.runs.truncate(start as usize + i.keep);
            if let (Some(cut), Some(last)) = (i.cut, self.runs.last_mut()) {
                last.text.end = cut;
            }
        }
        let runs = &self.runs[start as usize..];
        let src = match (implicit, runs.last()) {
            (_, None) => src,
            (true, Some(_)) => self.implicit_lines(start, src),
            // a hidden id on a line of its own leaves the paragraph the lines above it
            (false, Some(last)) if hidden.is_some() => {
                LineRange::new(src.start, src.end.min(last.src_line.max(src.start)))
            }
            (false, Some(_)) => src,
        };
        let block = (!runs.is_empty()).then(|| {
            self.embed(start, src)
                .unwrap_or_else(|| self.paragraph(start, src))
        });
        if let Some(i) = id {
            let (unit, lines) = match before {
                Some(named) => named,
                None => {
                    let b = block.as_ref()?;
                    (first_unit(b)?, self.item_lines().unwrap_or(b.src))
                }
            };
            self.anchors.push(Anchor {
                slug: format!("^{}", i.name).into(),
                text: i.name,
                kind: AnchorKind::Block { unit, lines },
            });
        }
        block
    }

    /// The lines of a tight item's text, an implicit frame that began at `start` and has runs:
    /// its runs' (the frame's own `src` is the line of its first run).
    fn implicit_lines(&self, start: u32, src: LineRange) -> LineRange {
        let runs = &self.runs[start as usize..];
        let first = runs.first().map_or(src.start, |r| r.src_line);
        let last = runs.last().map_or(src.end, |r| r.src_line);
        LineRange::new(first, last)
    }

    /// The block id `^id` that the runs from `start` on end in (OBS-009): after a blank, or
    /// alone on the paragraph's last line, of letters, digits and `-`, as Obsidian names a block.
    /// Plain text only, as written: not in code or a link, nor an escaped `\^`.
    /// Inlined into [`Builder::pop_block`], its one caller: LLVM took that call for a cold one
    /// and left it out of line, every paragraph a call (PAR-010).
    #[inline(always)]
    fn block_id(&self, start: u32) -> Option<BlockId> {
        let last = self.runs[start as usize..].last()?;
        // bytes, back from the end: most paragraphs end in a word or a stop, which no `^` heads
        // (a paragraph's check was ~170 instructions with `str`'s trims, the lists input +5%).
        // Loops of their own: `rposition`'s and `TextRange::of`'s calls stayed out of line,
        // ~55 instructions a paragraph (PAR-010)
        let bytes = &self.text.as_bytes()[last.text.start as usize..last.text.end as usize];
        let mut i = bytes.len();
        while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
            i -= 1;
        }
        let end = i;
        while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-') {
            i -= 1;
        }
        if i == 0 || i == end || bytes[i - 1] != b'^' {
            return None;
        }
        self.block_id_at(start, i - 1, end)
    }

    /// [`Builder::block_id`] past its quick test: the last run's text has a `^` at `caret`, then
    /// an id up to `end`.
    #[cold]
    fn block_id_at(&self, start: u32, caret: usize, end: usize) -> Option<BlockId> {
        let runs = &self.runs[start as usize..];
        let last = runs.last()?;
        if last.kind != RunKind::Text
            || last.link.is_some()
            || last.style.contains(InlineStyle::CODE)
        {
            return None;
        }
        let text = last.text.of(&self.text);
        // ASCII at `caret` and past it: char boundaries
        let (caret, name) = (&text[..caret], &text[caret + 1..end]);
        // the source's line says the `^` isn't escaped, and has a blank or the line's start
        // before it (a quote's `>`, a list's marker)
        let line = last.src_line as usize;
        let from = *self.line_starts.get(line.checked_sub(1)?)? as usize;
        let to = self
            .line_starts
            .get(line)
            .map_or(self.source.len(), |&s| s as usize);
        let written = self.source.get(from..to)?.trim_end();
        let before = written.strip_suffix(name)?.strip_suffix('^')?;
        if !before.is_empty() && !before.ends_with([' ', '\t', '>']) {
            return None;
        }
        // the runs left: those before the id's, the blanks and breaks before it gone
        let kept = caret.trim_end_matches([' ', '\t']);
        let (mut keep, mut cut) = match kept.is_empty() {
            true => (runs.len() - 1, None),
            false if kept.len() < caret.len() => {
                (runs.len(), Some(last.text.start + kept.len() as u32))
            }
            // `text^id`: no blank before the id
            false => return None,
        };
        if keep < runs.len() {
            // the id's run starts at the `^`: the run before it ends in a blank, as `**bold**^id`
            // doesn't, or is a break
            let blank = |r: &Run| {
                is_blank(r, &self.text)
                    || (r.kind == RunKind::Text && r.text.of(&self.text).ends_with([' ', '\t']))
            };
            if caret.is_empty() && keep > 0 && !blank(&runs[keep - 1]) {
                return None;
            }
            while keep > 0 && is_blank(&runs[keep - 1], &self.text) {
                keep -= 1;
            }
            if let Some(prev) = keep.checked_sub(1).map(|k| &runs[k])
                && prev.kind == RunKind::Text
            {
                let text = prev.text.of(&self.text);
                let trimmed = text.trim_end_matches([' ', '\t']).len();
                if trimmed < text.len() {
                    cut = Some(prev.text.start + trimmed as u32);
                }
            }
        }
        Some(BlockId {
            name: name.into(),
            keep,
            cut,
        })
    }

    /// The last block of the innermost container, the one a block id alone names.
    fn last_block(&self) -> Option<&Block> {
        self.stack.iter().rev().find_map(|f| match f {
            Frame::Blocks { blocks, .. } => Some(blocks.last()),
            _ => None,
        })?
    }

    /// The lines of the item whose first block is the one about to be pushed: a block id at the
    /// end of an item's text names the item, its children too.
    fn item_lines(&self) -> Option<LineRange> {
        match self.stack.last()? {
            Frame::Blocks {
                kind: Container::Item,
                blocks,
                src,
                ..
            } if blocks.is_empty() => Some(*src),
            _ => None,
        }
    }

    /// The paragraph of the inline frame just popped, which began at `start` and has runs. One
    /// that holds nothing but one image becomes an image block, and its runs go. Inlined, and
    /// the image looked for only past a quick test: v1.1.0's paragraph was inlined into
    /// [`Builder::pop_block`], and forced inline whole, its look went out of line (PAR-010).
    #[inline(always)]
    fn paragraph(&mut self, start: u32, src: LineRange) -> Block {
        let unit = self.next_unit();
        if self.may_be_image(start)
            && let Some(block) = self.image_block(start, src, unit)
        {
            return block;
        }
        Block {
            kind: BlockKind::Paragraph(Inlines {
                runs: self.close_runs(start),
                unit,
            }),
            src,
        }
    }

    /// Whether the runs from `start` on may be one image alone: not when the first is text that
    /// starts with printable ASCII, not a blank, as nearly every paragraph's does.
    #[inline(always)]
    fn may_be_image(&self, start: u32) -> bool {
        match self.runs.get(start as usize) {
            Some(r) if r.kind == RunKind::Text => !matches!(
                self.text.as_bytes().get(r.text.start as usize),
                Some(b'!'..=b'~')
            ),
            _ => true,
        }
    }

    /// [`Builder::paragraph`]'s image block, unit `unit`, when the runs from `start` on are one
    /// image alone, blanks aside.
    #[cold]
    #[inline(never)]
    fn image_block(&mut self, start: u32, src: LineRange, unit: UnitId) -> Option<Block> {
        let mut shown = self.runs[start as usize..]
            .iter()
            .filter(|r| !is_blank(r, &self.text));
        let (Some(run), None) = (shown.next(), shown.next()) else {
            return None;
        };
        let RunKind::InlineImage(link) = run.kind else {
            return None;
        };
        let alt = run.text.of(&self.text).into();
        self.runs.truncate(start as usize);
        Some(Block {
            kind: BlockKind::Image { link, alt, unit },
            src,
        })
    }

    /// The paragraph of the inline frame just popped, which began at `start`, as the note it
    /// embeds, when it holds nothing but the embed and the note was read (OBS-007): the embed's
    /// runs title the note's blocks. `None` leaves it a paragraph.
    fn embed(&mut self, start: u32, src: LineRange) -> Option<Block> {
        let e = self.embedding?;
        // another frame's runs after these would be split by the note's
        if self.inlines_open > 0 || e.stack.depth >= embed::MAX_DEPTH {
            return None;
        }
        let mut shown = self.runs[start as usize..]
            .iter()
            .filter(|r| !is_blank(r, &self.text));
        let (Some(run), None) = (shown.next(), shown.next()) else {
            return None;
        };
        let RunKind::Embed(link) = run.kind else {
            return None;
        };
        let Target::Local { target, frag } = Target::of(&self.links[link as usize]) else {
            return None;
        };
        let note = e.embeds.find(e.stack.path, &target)?;
        if e.stack.holds(&note.path) {
            return None;
        }
        let text = match &frag {
            Some(frag) => {
                let mut parsed = e.budget.parsed.borrow_mut();
                let at = match parsed.iter().position(|(p, _)| *p == note.path) {
                    Some(at) => at,
                    None => {
                        let doc = guard::catch(|| markdown(&note.text, e.dialect, None)).flatten();
                        parsed.push((note.path.clone(), doc));
                        parsed.len() - 1
                    }
                };
                section(&note.text, parsed[at].1.as_ref()?, frag)?
            }
            None => Cow::Borrowed(&*note.text),
        };
        let spent = e.budget.spent.get() + text.len();
        if spent > embed::MAX_TRANSCLUDED {
            return None;
        }
        e.budget.spent.set(spent);
        let stack = Stack {
            path: &note.path,
            up: Some(e.stack),
            depth: e.stack.depth + 1,
        };
        let inner = Embedding { stack: &stack, ..e };
        let parse = std::panic::AssertUnwindSafe(|| markdown(&text, e.dialect, Some(inner)));
        let note_doc = guard::catch(parse).flatten()?;
        let title = Inlines {
            unit: self.next_unit(),
            runs: self.close_runs(start),
        };
        let (blocks, note) = self.graft(note_doc, &note.path);
        Some(Block {
            kind: BlockKind::Embed(Box::new(Embed {
                title,
                blocks,
                note,
            })),
            src,
        })
    }

    /// Take `doc`'s IR, the note at `path`, into this one's: its text, runs, links, anchors,
    /// footnotes and math after this one's, its tables, code and search units numbered on from
    /// this one's. Its blocks are returned, with their place in [`Doc::embedded`]. Its footnotes
    /// join this one's, numbered in order of reference as they come; its front matter stays (an
    /// embed's layout hides it).
    fn graft(&mut self, doc: Doc, path: &Path) -> (Vec<Block>, u32) {
        let o = Offsets {
            text: self.text.len() as u32,
            runs: self.runs.len() as u32,
            links: self.links.len() as u32,
            anchors: self.anchors.len() as u32,
            footnotes: self.footnotes.len() as u32,
            math: self.math.len() as u32,
            tables: self.tables,
            codes: self.code_blocks,
            units: self.units,
            notes: self.embedded.len() as u32,
        };
        self.text.push_str(&doc.text);
        self.runs.extend(doc.runs.iter().map(|r| o.run(r)));
        self.math.extend(doc.math.iter().map(|&r| o.math(r)));
        let (links, anchors) = (doc.links.len() as u32, doc.anchors.len() as u32);
        self.links.extend(doc.links.into_iter().map(|l| o.link(l)));
        self.anchors
            .extend(doc.anchors.into_iter().map(|a| o.anchor(a)));
        // in the order the note numbered them, which joins the host's order of reference
        let mut order: Vec<(u32, FootnoteId)> = Vec::new();
        for (i, note) in doc.footnotes.into_iter().enumerate() {
            order.push((note.number, o.footnotes + i as FootnoteId));
            self.footnotes.push(o.footnote(note));
        }
        order.sort_unstable();
        self.ref_order.extend(order.into_iter().map(|(_, id)| id));
        self.tables += doc.table_count;
        self.code_blocks += doc.code_count;
        self.units += doc.unit_count;
        self.embedded
            .extend(doc.embedded.into_iter().map(|e| o.embedded(e)));
        let note = self.embedded.len() as u32;
        self.embedded.push(Embedded {
            path: path.to_owned(),
            links: Slots {
                start: o.links,
                end: o.links + links,
            },
            anchors: Slots {
                start: o.anchors,
                end: o.anchors + anchors,
            },
        });
        let mut blocks = doc.blocks;
        blocks.retain(|b| !matches!(b.kind, BlockKind::FootnoteDefs));
        o.blocks(&mut blocks);
        (blocks, note)
    }

    fn next_unit(&mut self) -> UnitId {
        self.units += 1;
        self.units - 1
    }

    #[inline]
    fn push_block(&mut self, block: Block) {
        self.close_implicit();
        let target = self.stack.iter_mut().rev().find_map(|f| match f {
            Frame::Blocks { blocks, .. } => Some(blocks),
            _ => None,
        });
        if let Some(blocks) = target {
            blocks.push(block);
        }
    }

    fn finish(mut self) -> Doc {
        while self.stack.len() > 1 {
            self.close_implicit();
            self.pop_block();
        }
        let mut blocks = match self.stack.pop() {
            Some(Frame::Blocks { blocks, .. }) => blocks,
            _ => Vec::new(),
        };

        // Number footnotes in order of first reference, then any that are never referenced.
        let mut next = 0;
        let mut numbers = vec![0; self.footnotes.len()];
        for id in self
            .ref_order
            .iter()
            .copied()
            .chain(0..self.footnotes.len() as u32)
        {
            let n = &mut numbers[id as usize];
            if *n == 0 {
                next += 1;
                *n = next;
            }
        }
        for (note, n) in self.footnotes.iter_mut().zip(numbers) {
            note.number = n;
        }
        if self.footnotes.iter().any(|f| f.anchor.is_some()) {
            blocks.push(Block {
                kind: BlockKind::FootnoteDefs,
                src: LineRange::NONE,
            });
        }

        self.text.shrink_to_fit();
        self.runs.shrink_to_fit();
        Doc {
            blocks,
            text: self.text,
            runs: self.runs,
            math: self.math,
            links: self.links,
            anchors: self.anchors,
            footnotes: self.footnotes,
            embedded: self.embedded,
            line_starts: self.line_starts,
            table_count: self.tables,
            code_count: self.code_blocks,
            unit_count: self.units,
        }
    }
}

/// A block id at the end of a paragraph's runs (OBS-009), and the runs left without it.
struct BlockId {
    name: Box<str>,
    /// How many of the paragraph's runs are left: 0 for the id alone.
    keep: usize,
    /// Where the last of them now ends in [`Builder::text`], past the text before the id.
    cut: Option<u32>,
}

/// The first search unit of `block`, by which layout finds the row a block id names (OBS-009):
/// none for a block without text of its own, as a rule.
fn first_unit(block: &Block) -> Option<UnitId> {
    match &block.kind {
        BlockKind::Heading { inlines, .. }
        | BlockKind::Paragraph(inlines)
        | BlockKind::Math(inlines) => Some(inlines.unit),
        BlockKind::List(list) => list
            .items
            .iter()
            .find_map(|i| i.blocks.iter().find_map(first_unit)),
        BlockKind::Quote { callout, blocks } => match callout {
            Some(c) => Some(c.title.unit),
            None => blocks.iter().find_map(first_unit),
        },
        BlockKind::Code(code) => Some(code.first_unit),
        BlockKind::Table(table) => table.cells.first().map(|c| c.unit),
        BlockKind::Image { unit, .. } => Some(*unit),
        BlockKind::Embed(e) => Some(e.title.unit),
        BlockKind::Rule
        | BlockKind::Html(_)
        | BlockKind::FootnoteDefs
        | BlockKind::Properties(_) => None,
    }
}

fn is_blank(run: &Run, text: &str) -> bool {
    match run.kind {
        RunKind::Text => run.text.of(text).trim().is_empty(),
        RunKind::SoftBreak | RunKind::HardBreak => true,
        RunKind::Math(_)
        | RunKind::InlineImage(_)
        | RunKind::Embed(_)
        | RunKind::FootnoteRef(_) => false,
    }
}

/// Whether `c` may be in an Obsidian tag's name: anything but a space, ASCII punctuation other
/// than `-`, `_` and `/`, and Unicode's general and supplemental punctuation, as Obsidian reads
/// tags; past Obsidian, CJK punctuation and the full-width forms of ASCII's end a tag too, so
/// `#读书，` is `#读书`. Letters of any script, digits and emoji are in.
fn is_tag_char(c: char) -> bool {
    let punct = |c: char| c.is_ascii_punctuation() && !matches!(c, '-' | '_' | '/');
    let full_width = (c as u32).checked_sub(0xFEE0).and_then(char::from_u32);
    !(c.is_whitespace()
        || punct(c)
        || ('\u{2000}'..='\u{206F}').contains(&c)
        || ('\u{2E00}'..='\u{2E7F}').contains(&c)
        || matches!(c, '\u{3001}'..='\u{3003}' | '\u{3008}'..='\u{3011}' | '\u{3014}'..='\u{301F}')
        || matches!(
            c,
            '\u{3030}' | '\u{303D}' | '\u{30FB}' | '\u{FF5F}'..='\u{FF65}'
        )
        || (('\u{FF01}'..='\u{FF5E}').contains(&c) && full_width.is_some_and(punct)))
}

fn is_break(run: &Run) -> bool {
    matches!(run.kind, RunKind::SoftBreak | RunKind::HardBreak)
}

/// The part of the note `text`, parsed as `doc`, that `frag` names, as Obsidian embeds it: a
/// heading's section, from the heading's line to the line before the next heading of its level
/// or above, at the note's top level; or a block's, `^id` (OBS-009), its lines read as the block
/// alone. `None` when nothing has the name.
fn section<'t>(text: &'t str, doc: &Doc, frag: &str) -> Option<Cow<'t, str>> {
    let anchor = crate::links::find_anchor(doc, frag)?;
    if let AnchorKind::Block { lines, .. } = doc.anchors.get(anchor as usize)?.kind {
        return Some(outdent(embed::lines(text, lines.start, Some(lines.end))));
    }
    let heading = |b: &Block| match b.kind {
        BlockKind::Heading { level, anchor, .. } => Some((level, anchor, b.src.start)),
        _ => None,
    };
    let mut headings = doc.blocks.iter().filter_map(heading);
    let (level, _, first) = headings.find(|&(_, a, _)| a == anchor)?;
    let next = headings
        .find(|&(l, _, _)| l <= level)
        .map(|(_, _, line)| line - 1);
    Some(Cow::Borrowed(embed::lines(text, first, next)))
}

/// `lines` of a block nested in a list or quote, without the blanks and quote marks (`> `) its
/// first line has before it, as far as each line has them: the block alone, so a list item
/// four spaces in isn't read as code, and a quote's paragraph is a paragraph.
fn outdent(lines: &str) -> Cow<'_, str> {
    let prefix = |line: &str| {
        let bytes = line
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'>'));
        bytes.count()
    };
    let width = prefix(lines);
    if width == 0 {
        return Cow::Borrowed(lines);
    }
    let mut out = String::with_capacity(lines.len());
    for line in lines.split_inclusive('\n') {
        out.push_str(&line[prefix(line).min(width)..]);
    }
    Cow::Owned(out)
}

/// Whether an image's `dest` embeds a note or another file that isn't an image (OBS-007), shown
/// as a link: a wiki embed (`![[note]]`, `![[note#Heading]]`, `![[file.pdf]]`) of anything but an
/// image, or a local `![](note.md)` or `![](file.pdf)`. An image, a URL and a path of no other
/// known kind stay images.
fn is_embed(dest: &str, link_type: LinkType) -> bool {
    use crate::links;
    use std::path::Path;
    let path = Path::new(dest.split('#').next().unwrap_or_default());
    match link_type {
        LinkType::WikiLink { .. } => !links::is_image(path),
        _ => {
            links::scheme(dest).is_none()
                && (links::is_markdown(path) || links::has_extension(path, &["pdf"]))
        }
    }
}

/// What of a text event at `start` comes at or after `title`, where a callout's title starts:
/// the text before it is the marker's and the spaces after it, so its bytes are the source's.
fn title_text(text: &str, start: usize, title: usize) -> Option<Event<'static>> {
    let rest = text
        .get((title - start).min(text.len())..)
        .unwrap_or_default();
    match rest.trim_start() {
        "" => None,
        rest => Some(Event::Text(rest.to_owned().into())),
    }
}

fn gfm_name(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => "note",
        BlockQuoteKind::Tip => "tip",
        BlockQuoteKind::Important => "important",
        BlockQuoteKind::Warning => "warning",
        BlockQuoteKind::Caution => "caution",
    }
}

/// The callout marker on the first line of the quote whose `>` is at `at` (OBS-002): `[!kind]`
/// after the `>` and its spaces, the kind of letters, digits, `-` and `_`, then `+` or `-`, then
/// the title. Only offsets: the parser's events confirm it ([`Builder::read_marker`]). Inlined
/// up to the `[!`, which most quotes lack: a call at every quote was ~55 instructions (PAR-010).
#[inline(always)]
fn marker(source: &str, at: usize, obsidian: bool) -> Option<Marker> {
    let b = source.as_bytes();
    let mut i = at;
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if b.get(i) != Some(&b'>') {
        return None;
    }
    i += 1;
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if b.get(i..i + 2) != Some(b"[!") {
        return None;
    }
    marker_at(source, i, obsidian)
}

/// [`marker`] past its quick test: the `[!` at `open`.
#[inline(never)]
fn marker_at(source: &str, open: usize, obsidian: bool) -> Option<Marker> {
    let b = source.as_bytes();
    let blank = |i: &mut usize| {
        while b.get(*i).is_some_and(|&c| c == b' ' || c == b'\t') {
            *i += 1;
        }
    };
    let mut i = open + 2;
    let name = i;
    while b
        .get(i)
        .is_some_and(|&c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        i += 1;
    }
    if i == name || b.get(i) != Some(&b']') {
        return None;
    }
    let name = name..i;
    i += 1;
    let fold = match b.get(i) {
        Some(b'+') => Some(Fold::Open),
        Some(b'-') => Some(Fold::Folded),
        _ => None,
    };
    i += usize::from(fold.is_some());
    let end = i;
    blank(&mut i);
    Some(Marker {
        kind: CalloutKind::of(&source[name.clone()], obsidian),
        name,
        fold,
        at: open,
        end,
        title: i,
        state: MarkerState::Armed,
    })
}

/// What opens a quote or an item, being read.
enum Opening {
    Callout(Marker),
    Status(Status),
}

/// A task status past GFM's after an item's list marker (OBS-012), being read.
struct Status {
    task: Task,
    /// Where the next of its text events starts, from the `[` on.
    next: usize,
    /// Past its `]`.
    end: usize,
    /// Whether its first text came: the item is a task.
    read: bool,
}

/// The task status past GFM's after the list marker of the item at `item`: one of
/// [`Task::STATUSES`] in brackets, then a space or a tab, as GFM's own `[x]` is written. A `[*]`
/// only in an item with no other `*`, which could pair with it into emphasis. Inlined up to the
/// `[`, which most items lack: a call at every item was ~60 instructions (PAR-010).
#[inline(always)]
fn status(source: &str, item: Range<usize>) -> Option<Status> {
    let b = source.as_bytes();
    let mut i = item.start;
    match *b.get(i)? {
        b'-' | b'*' | b'+' => i += 1,
        c if c.is_ascii_digit() => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if !matches!(b.get(i), Some(b'.' | b')')) {
                return None;
            }
            i += 1;
        }
        _ => return None,
    }
    let marker = i;
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    // GFM's own `[ ]` and `[x]` are pulldown-cmark's
    if i == marker || b.get(i) != Some(&b'[') || matches!(b.get(i + 1), Some(b' ' | b'x' | b'X')) {
        return None;
    }
    status_at(source, item, i)
}

/// [`status`] past its quick test: a `[` at `i`, after the marker and a blank, not GFM's.
#[inline(never)]
fn status_at(source: &str, item: Range<usize>, i: usize) -> Option<Status> {
    let b = source.as_bytes();
    let boxed = b.get(i + 2) == Some(&b']') && matches!(b.get(i + 3), Some(b' ' | b'\t'));
    let c = char::from(*b.get(i + 1).filter(|_| boxed)?);
    let star = c == '*'
        && source
            .get(i + 3..item.end)
            .is_none_or(|rest| rest.contains('*'));
    (Task::STATUSES.contains(&c) && !star).then_some(Status {
        task: Task::Status(c),
        next: i,
        end: i + 3,
        read: false,
    })
}

/// A `==` in a text run that may open or close a highlight (OBS-005), and which it does once
/// paired.
struct Delim {
    /// Its run, from the closing frame's first.
    run: usize,
    /// Where it is in [`Builder::text`].
    at: u32,
    open: bool,
    close: bool,
    side: Option<Side>,
}

#[derive(Clone, Copy)]
enum Side {
    Open,
    Close,
}

/// The character `run` shows at its end (`last`) or start, for a `==` next to it: a break as a
/// space, as the frame's edge (`None`) is; code, an image, an embed or a footnote reference as
/// the punctuation around it in the source.
fn edge(run: Option<&Run>, text: &str, last: bool) -> char {
    match run.map(|r| (r, r.kind)) {
        Some((r, RunKind::Text)) if !r.style.contains(InlineStyle::CODE) => {
            let t = r.text.of(text);
            match last {
                true => t.chars().next_back(),
                false => t.chars().next(),
            }
            .unwrap_or(' ')
        }
        None | Some((_, RunKind::SoftBreak | RunKind::HardBreak)) => ' ',
        Some(_) => '`',
    }
}

/// Whether a delimiter run between `before` and `after` is left-flanking (may open) and
/// right-flanking (may close), as CommonMark has it.
fn flanking(before: char, after: char) -> (bool, bool) {
    let punct = |c: char| c.is_ascii_punctuation();
    let left = !after.is_whitespace() && (!punct(after) || before.is_whitespace() || punct(before));
    let right =
        !before.is_whitespace() && (!punct(before) || after.is_whitespace() || punct(after));
    (left, right)
}

fn align(a: Alignment) -> Align {
    match a {
        Alignment::None => Align::None,
        Alignment::Left => Align::Left,
        Alignment::Center => Align::Center,
        Alignment::Right => Align::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How [`markdown`] masks `$`s, in tests.
    #[derive(Clone, Copy, PartialEq)]
    enum Masks {
        /// As it does.
        Shipped,
        /// Without [`never_opens`]' masks: the passes alone.
        PassesOnly,
        /// pandoc's rule, the reference: passes alone, each masking only its first false
        /// closer, as many as it takes. Left to right, a pass's masks are what pandoc skips;
        /// the shipped passes mask the same ones, more of them a pass ([`Builder::settle`]).
        Ideal,
    }

    thread_local! {
        /// [`markdown`] finds an Obsidian note's code by a pass of its own, whatever its `%%`s
        /// look like (PAR-021).
        pub(super) static CODE_PASS: Cell<bool> = const { Cell::new(false) };
    }

    thread_local! {
        static MASKS: Cell<Masks> = const { Cell::new(Masks::Shipped) };
        /// What [`never_opens`] gave the last parse.
        static NEVER: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
        /// The passes [`markdown`] made, counted.
        pub(super) static PASSES: Cell<usize> = const { Cell::new(0) };
        /// The mask of the last pass [`markdown`] made.
        pub(super) static MASK: Cell<Option<u8>> = const { Cell::new(None) };
        /// The false closers each pass met, in the text [`markdown`] parsed.
        pub(super) static WRONG: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
        /// The false closers [`markdown`]'s last parse kept ([`kept_besides`]), counted.
        pub(super) static KEPT: Cell<usize> = const { Cell::new(0) };
        /// [`markdown`]'s last parse met a false closer in its last pass
        /// ([`MAX_MATH_PASSES`]), which read it as pulldown-cmark does.
        pub(super) static CAPPED: Cell<bool> = const { Cell::new(false) };
    }

    /// [`markdown`]'s `never` as [`MASKS`] has it.
    pub(super) fn hooked(never: Vec<usize>) -> Vec<usize> {
        NEVER.set(never.clone());
        match MASKS.get() {
            Masks::Shipped => never,
            Masks::PassesOnly | Masks::Ideal => Vec::new(),
        }
    }

    /// Whether [`markdown`] parses as [`Masks::Ideal`].
    pub(super) fn ideal() -> bool {
        MASKS.get() == Masks::Ideal
    }

    thread_local! {
        /// [`markdown`] masks with a control byte ([`mask_byte`]), as in a note that uses every
        /// punctuation byte.
        static CONTROL: Cell<bool> = const { Cell::new(false) };
    }

    thread_local! {
        /// [`markdown`] masks with this byte ([`mask_byte`]), one the test made sure the text
        /// lacks.
        static FORCE: Cell<Option<u8>> = const { Cell::new(None) };
    }

    /// The byte [`mask_byte`] takes for `text` under [`FORCE`], or the control byte under
    /// [`CONTROL`].
    pub(super) fn forced(text: &str) -> Option<u8> {
        if let Some(f) = FORCE.get() {
            return Some(f);
        }
        CONTROL.get().then(|| {
            (1..32u8).find(|&c| !matches!(c, b'\t'..=b'\r') && !text.as_bytes().contains(&c))
        })?
    }

    fn kinds(blocks: &[Block]) -> Vec<&'static str> {
        blocks
            .iter()
            .map(|b| match &b.kind {
                BlockKind::Heading { .. } => "heading",
                BlockKind::Paragraph(_) => "paragraph",
                BlockKind::List(_) => "list",
                BlockKind::Quote { .. } => "quote",
                BlockKind::Code(_) => "code",
                BlockKind::Table(_) => "table",
                BlockKind::Image { .. } => "image",
                BlockKind::Math(_) => "math",
                BlockKind::Rule => "rule",
                BlockKind::Html(_) => "html",
                BlockKind::FootnoteDefs => "footnotes",
                BlockKind::Properties(_) => "properties",
                BlockKind::Embed(_) => "embed",
            })
            .collect()
    }

    fn list(b: &Block) -> &List {
        match &b.kind {
            BlockKind::List(l) => l,
            _ => panic!("not a list"),
        }
    }

    /// A paragraph's or heading's text.
    fn shown(doc: &Doc, b: &Block) -> String {
        match &b.kind {
            BlockKind::Paragraph(inlines) | BlockKind::Heading { inlines, .. } => {
                doc.plain_text(inlines)
            }
            _ => panic!("no inlines"),
        }
    }

    #[test]
    fn wikilinks_with_empty_text_stay_markdown() {
        // pulldown-cmark#1102, fixed in 0.13.5: each `[[a|]]` doubled the events, so 40 of them
        // went over `max_events` and the text showed as plain
        let text = "[[a|]] ".repeat(40);
        let doc = parse(&text);
        assert_eq!(kinds(&doc.blocks), ["paragraph"]);
        assert_eq!(doc.links.len(), 40);
    }

    #[test]
    fn a_blank_line_after_a_link_reference_item_stays_markdown() {
        // pulldown-cmark#1129, fixed in 0.13.5: `into_offset_iter` unwrapped an empty tight
        // paragraph, and the caught panic showed the text as plain
        for (text, kind) in [
            ("- [o]:u\n    \t", "list"),
            ("> - [o]:u\n>     \t", "quote"),
            ("+\t[\0]:d\n\u{b}", "list"),
        ] {
            let doc = parse(text);
            assert_eq!(kinds(&doc.blocks), [kind], "{text:?}");
        }
    }

    #[test]
    fn runs_hold_their_text_as_ranges_of_the_docs() {
        let doc = parse(
            "# Head `x`\n\na ![alt *em*\nnext](i.png) b[^n]\\\nc<br>d\n\n![lone](j.png)\n\n[^n]: note\n",
        );
        let BlockKind::Heading { inlines, .. } = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(doc.plain_text(inlines), "Head x");
        assert_eq!(&*doc.anchors[0].text, "Head x");
        let BlockKind::Paragraph(inl) = &doc.blocks[1].kind else {
            panic!()
        };
        let runs: Vec<_> = doc
            .runs(inl)
            .iter()
            .map(|r| (doc.run_text(r), r.kind))
            .collect();
        assert_eq!(
            runs,
            [
                ("a ", RunKind::Text),
                // the alt's markup and break flattened, as it was written into the text
                ("alt em next", RunKind::InlineImage(0)),
                (" b", RunKind::Text),
                ("", RunKind::FootnoteRef(0)),
                ("", RunKind::HardBreak),
                ("c", RunKind::Text),
                ("", RunKind::HardBreak),
                ("d", RunKind::Text),
            ]
        );
        assert!(matches!(&doc.blocks[2].kind, BlockKind::Image { alt, .. } if &**alt == "lone"));
        let BlockKind::Paragraph(note) = &doc.footnotes[0].blocks[0].kind else {
            panic!()
        };
        assert_eq!(doc.plain_text(note), "note");
    }

    #[test]
    fn each_paragraphs_and_cells_runs_are_one_range_of_the_docs() {
        let doc =
            parse("a *b*\n\n![lone](i.png)\n\n| h | i |\n|---|---|\n| 1 | |\n| 3 | 4 | 5 |\n\nc\n");
        let texts = |inl: &Inlines| -> Vec<&str> {
            doc.runs(inl).iter().map(|r| doc.run_text(r)).collect()
        };
        let BlockKind::Paragraph(a) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(texts(a), ["a ", "b"]);
        let BlockKind::Table(t) = &doc.blocks[2].kind else {
            panic!()
        };
        let row = |r: &TableRow| -> Vec<String> {
            t.cells(r).iter().map(|c| doc.plain_text(c)).collect()
        };
        assert_eq!(row(&t.header), ["h", "i"]);
        assert_eq!(row(&t.rows[0]), ["1", ""]);
        // a cell past the alignment row's columns is dropped by pulldown-cmark
        assert_eq!(row(&t.rows[1]), ["3", "4"]);
        assert_eq!(t.cells.len(), 6);
        let BlockKind::Paragraph(c) = &doc.blocks[3].kind else {
            panic!()
        };
        assert_eq!(texts(c), ["c"]);
        // the lone image's runs went with its paragraph
        assert_eq!(doc.runs.len(), 2 + 5 + 1);
    }

    #[test]
    fn an_inline_frame_inside_another_keeps_both_ranges_whole() {
        // pulldown-cmark doesn't nest them; the builder mustn't depend on it
        let mut b = Builder::new("");
        b.push_inline(InlineKind::Paragraph, LineRange::line(1));
        b.push_text("a", InlineStyle::NONE, 1);
        b.push_inline(InlineKind::Paragraph, LineRange::line(1));
        b.push_text("b", InlineStyle::NONE, 1);
        b.push_text("c", InlineStyle::EMPH, 1);
        b.pop_block();
        b.push_text("d", InlineStyle::NONE, 1);
        b.pop_block();
        let doc = b.finish();
        let texts: Vec<Vec<&str>> = (doc.blocks.iter())
            .map(|block| match &block.kind {
                BlockKind::Paragraph(inl) => {
                    doc.runs(inl).iter().map(|r| doc.run_text(r)).collect()
                }
                _ => panic!(),
            })
            .collect();
        assert_eq!(texts, [vec!["b", "c"], vec!["a", "d"]]);
    }

    #[test]
    fn inline_markup_is_resolved_into_runs() {
        let doc = parse("some **bold** and _emph_ and `code`\n");
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        let runs: Vec<_> = doc
            .runs(inl)
            .iter()
            .map(|r| (doc.run_text(r), r.style))
            .collect();
        assert_eq!(
            runs,
            [
                ("some ", InlineStyle::NONE),
                ("bold", InlineStyle::STRONG),
                (" and ", InlineStyle::NONE),
                ("emph", InlineStyle::EMPH),
                (" and ", InlineStyle::NONE),
                ("code", InlineStyle::CODE),
            ]
        );
    }

    #[test]
    fn line_ranges() {
        let doc = parse("# Title\n\npara one\npara two\n\n```rust\nfn x() {}\n```\n");
        assert_eq!(kinds(&doc.blocks), ["heading", "paragraph", "code"]);
        assert_eq!(doc.blocks[0].src, LineRange::new(1, 1));
        assert_eq!(doc.blocks[1].src, LineRange::new(3, 4));
        assert_eq!(doc.blocks[2].src, LineRange::new(6, 8));
        let BlockKind::Code(code) = &doc.blocks[2].kind else {
            panic!()
        };
        assert_eq!(code.first_line, 7);
        assert_eq!(code.lang.as_deref(), Some("rust"));
        assert_eq!(code.lines().collect::<Vec<_>>(), ["fn x() {}"]);
        let BlockKind::Paragraph(inl) = &doc.blocks[1].kind else {
            panic!()
        };
        assert_eq!(doc.runs(inl).last().map(|r| r.src_line), Some(4));
    }

    #[test]
    fn lines_found_forward_match_a_search() {
        let text = "a\n\nbb\nccc\n\n\n\ndddd\ne\nf\ng\nh\ni\nj\nk\nl\nm\n";
        let starts = super::super::lines::line_starts(text);
        let search = |offset: usize| starts.partition_point(|&s| s as usize <= offset) as u32;
        for offset in 0..=text.len() {
            for line in 1..=search(offset) {
                assert_eq!(
                    line_from(&starts, line, offset),
                    search(offset),
                    "{line} {offset}"
                );
            }
        }
        // the cursor, offsets in any order
        let mut b = Builder::new(text);
        for offset in [0, 5, 3, 3, 20, 4, 0, text.len(), 1, 9, 9, 30] {
            assert_eq!(b.line_of(offset), search(offset), "{offset}");
        }
    }

    #[test]
    fn tight_and_loose_lists() {
        let doc = parse("- a\n- b\n  - nested\n\n1. x\n\n2. y\n");
        assert_eq!(kinds(&doc.blocks), ["list", "list"]);
        let tight = list(&doc.blocks[0]);
        assert!(tight.tight);
        assert_eq!(kinds(&tight.items[1].blocks), ["paragraph", "list"]);
        let loose = list(&doc.blocks[1]);
        assert!(!loose.tight);
        assert_eq!(loose.start, Some(1));
    }

    #[test]
    fn task_items() {
        let doc = parse("- [x] done\n- [ ] open\n- plain\n");
        let tasks: Vec<_> = list(&doc.blocks[0]).items.iter().map(|i| i.task).collect();
        assert_eq!(tasks, [Some(Task::Done), Some(Task::Open), None]);
    }

    #[test]
    fn obsidians_task_statuses_are_tasks() {
        let md = "- [/] in *progress*\n- [-]\tcancelled\n1. [>] forwarded\n- [!] ^not-an-id\n\
                  - [<] \n- [?] a\n  - [*] star\n\n  [/] not the item's start\n";
        let doc = parse(md);
        let items = |b: &Block| -> Vec<(Option<Task>, String)> {
            let first = |i: &ListItem| i.blocks.first().map(|b| shown(&doc, b)).unwrap_or_default();
            list(b).items.iter().map(|i| (i.task, first(i))).collect()
        };
        let status = |c| Some(Task::Status(c));
        assert_eq!(
            items(&doc.blocks[0]),
            [
                (status('/'), "in progress".into()),
                (status('-'), "cancelled".into()),
            ]
        );
        assert_eq!(items(&doc.blocks[1]), [(status('>'), "forwarded".into())]);
        let last = items(&doc.blocks[2]);
        assert_eq!(last[0], (status('!'), "^not-an-id".into()));
        assert_eq!(last[1], (status('<'), String::new()));
        assert_eq!(last[2], (status('?'), "a".into()));
        let nested = &list(&doc.blocks[2]).items[2].blocks;
        assert_eq!(items(&nested[1]), [(status('*'), "star".into())]);
        assert_eq!(shown(&doc, &nested[2]), "[/] not the item's start");

        // not a status: a letter or a digit, no blank after it, a `*` that may be emphasis, a
        // reference, escaped, GFM's own
        let md = "- [a] option\n- [1] Knuth\n- [/]x\n- [*] *emphasis*\n- [/]\n- \\[/] escaped\n\
                  - [X] done\n\n[/]: https://example.com\n";
        let doc = parse(md);
        let tasks: Vec<_> = list(&doc.blocks[0]).items.iter().map(|i| i.task).collect();
        assert_eq!(
            tasks,
            [None, None, None, None, None, None, Some(Task::Done)]
        );
        let doc = parse("- [/] a reference\n\n[/]: https://example.com\n");
        assert_eq!(list(&doc.blocks[0]).items[0].task, None);
        assert_eq!(doc.links.len(), 1);
    }

    #[test]
    fn footnotes_are_numbered_by_first_reference() {
        let doc = parse("b[^b] a[^a] b again[^b]\n\n[^a]: first def\n[^b]: second def\n");
        assert_eq!(kinds(&doc.blocks), ["paragraph", "footnotes"]);
        let numbers: Vec<_> = doc
            .footnotes
            .iter()
            .map(|f| (&*f.label, f.number))
            .collect();
        assert_eq!(numbers, [("b", 1), ("a", 2)]);
        assert!(doc.footnotes.iter().all(|f| f.anchor.is_some()));
    }

    #[test]
    fn lone_image_becomes_an_image_block() {
        let doc = parse("![a cat](cat.png)\n\ntext ![inline](x.png) text\n");
        assert_eq!(kinds(&doc.blocks), ["image", "paragraph"]);
        assert_eq!(&*doc.links[0].dest, "cat.png");
    }

    #[test]
    fn wiki_links() {
        let doc = parse("see [[other-note#Some Heading|alias]] and [[plain]]\n");
        let kinds: Vec<_> = doc.links.iter().map(|l| (&*l.dest, l.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("other-note#Some Heading", LinkKind::Wiki { aliased: true }),
                ("plain", LinkKind::Wiki { aliased: false }),
            ]
        );
    }

    #[test]
    fn headings_get_unique_anchors() {
        let doc = parse("# Intro\n## Intro\n## `code` *x*\n");
        let slugs: Vec<_> = doc.anchors.iter().map(|a| &*a.slug).collect();
        assert_eq!(slugs, ["intro", "intro-1", "code-x"]);
    }

    #[test]
    fn tables() {
        let doc = parse("| a | b |\n|---|--:|\n| 1 | 2 |\n| 3 | 4 |\n");
        let BlockKind::Table(t) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(t.aligns, [Align::None, Align::Right]);
        assert_eq!(t.header.cells.len(), 2);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.header.src, LineRange::line(1));
        assert_eq!(t.rows[1].src, LineRange::line(4));
    }

    #[test]
    fn callouts_and_comments() {
        let doc = parse("> [!WARNING]\n> careful\n\n<!-- hidden -->\n\n<div>shown</div>\n");
        assert_eq!(kinds(&doc.blocks), ["quote", "html"]);
        assert!(matches!(
            doc.blocks[0].kind,
            BlockKind::Quote {
                callout: Some(Callout {
                    kind: CalloutKind::Warning,
                    ..
                }),
                ..
            }
        ));
    }

    /// A callout as kind, title, fold and the kinds of its blocks.
    type Shown = (CalloutKind, String, Option<Fold>, Vec<&'static str>);

    /// Each top-level quote's callout.
    fn callouts(doc: &Doc) -> Vec<Option<Shown>> {
        (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Quote { callout, blocks } => Some(callout.as_ref().map(|c| {
                    let title = plain_text(doc.runs(&c.title), &doc.text);
                    (c.kind, title, c.fold, kinds(blocks))
                })),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn obsidians_callouts_have_kinds_titles_and_folds() {
        let doc = parse(
            "> [!info] A *styled* title\n> body\n> more\n\n\
             > [!faq]- Folded?\n> yes\n\n\
             > [!TLDR]+\n> short\n\n\
             > [!theorem]  Pythagoras\n\n\
             > [!done]\n",
        );
        use CalloutKind as K;
        let para = vec!["paragraph"];
        assert_eq!(
            callouts(&doc),
            [
                Some((K::Info, "A styled title".into(), None, para.clone())),
                Some((
                    K::Question,
                    "Folded?".into(),
                    Some(Fold::Folded),
                    para.clone()
                )),
                Some((K::Abstract, "Tldr".into(), Some(Fold::Open), para)),
                // an unknown kind is a note, titled as written
                Some((K::Note, "Pythagoras".into(), None, vec![])),
                Some((K::Success, "Done".into(), None, vec![])),
            ]
        );
        // the title keeps its markup, and the body starts on the next line
        let BlockKind::Quote {
            callout: Some(c),
            blocks,
        } = &doc.blocks[0].kind
        else {
            panic!()
        };
        let styles: Vec<_> = doc.runs(&c.title).iter().map(|r| r.style).collect();
        assert_eq!(
            styles,
            [InlineStyle::NONE, InlineStyle::EMPH, InlineStyle::NONE]
        );
        assert_eq!(blocks[0].src, LineRange::new(2, 3));
        let BlockKind::Paragraph(body) = &blocks[0].kind else {
            panic!()
        };
        assert_eq!(plain_text(doc.runs(body), &doc.text), "body more");
    }

    #[test]
    fn githubs_alerts_are_callouts_and_obsidian_reads_two_as_aliases() {
        let text = "> [!IMPORTANT]\n> a\n\n> [!caution]\n> b\n\n> [!Note] titled\n> c\n";
        use CalloutKind as K;
        let kinds_in = |dialect| -> Vec<_> {
            (callouts(&parse_as(text, dialect)).into_iter())
                .map(|c| c.map(|(kind, title, ..)| (kind, title)))
                .collect()
        };
        assert_eq!(
            kinds_in(Dialect::CommonMark),
            [
                Some((K::Important, "Important".into())),
                Some((K::Caution, "Caution".into())),
                Some((K::Note, "titled".into())),
            ]
        );
        assert_eq!(
            kinds_in(Dialect::Obsidian { line_breaks: true }),
            [
                Some((K::Tip, "Important".into())),
                Some((K::Warning, "Caution".into())),
                Some((K::Note, "titled".into())),
            ]
        );
    }

    #[test]
    fn nested_callouts_and_quotes_that_arent_callouts() {
        let doc = parse("> [!question] Nested?\n> > [!todo] Yes\n> > > [!example] Deeper\n");
        let BlockKind::Quote {
            callout: Some(c),
            blocks,
        } = &doc.blocks[0].kind
        else {
            panic!()
        };
        assert_eq!(c.kind, CalloutKind::Question);
        let BlockKind::Quote {
            callout: Some(todo),
            blocks,
        } = &blocks[0].kind
        else {
            panic!()
        };
        assert_eq!(todo.kind, CalloutKind::Todo);
        assert!(matches!(
            &blocks[0].kind,
            BlockKind::Quote {
                callout: Some(Callout {
                    kind: CalloutKind::Example,
                    ..
                }),
                ..
            }
        ));
        // not on the first line, not first on it, indented into code, a bad kind, a link
        for text in [
            "> text\n> [!info] later\n",
            "> text [!info] x\n",
            ">     [!info] code\n",
            "> [!in fo] x\n",
            "> [!info](https://x.y) x\n",
            "> [!info]\n\n[!info]: https://x.y\n",
        ] {
            let doc = parse(text);
            assert_eq!(callouts(&doc), [None], "{text:?}");
        }
        // a marker cut by a backslash: the text as written
        let doc = parse("> [!in\\fo] x\n");
        assert_eq!(callouts(&doc), [None]);
    }

    /// Each top-level paragraph's runs, as text and kind.
    fn paragraph_runs(doc: &Doc) -> Vec<Vec<(&str, RunKind)>> {
        (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Paragraph(inl) => Some(
                    (doc.runs(inl).iter())
                        .map(|r| (doc.run_text(r), r.kind))
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn obsidians_single_newlines_break_the_line() {
        let text = "one\ntwo\n\n- a\n  b\n";
        let shown = Dialect::Obsidian { line_breaks: true };
        let strict = Dialect::Obsidian { line_breaks: false };
        let doc = parse_as(text, shown);
        assert_eq!(
            paragraph_runs(&doc),
            [vec![
                ("one", RunKind::Text),
                ("", RunKind::HardBreak),
                ("two", RunKind::Text)
            ]]
        );
        let BlockKind::List(list) = &doc.blocks[1].kind else {
            panic!()
        };
        let BlockKind::Paragraph(item) = &list.items[0].blocks[0].kind else {
            panic!()
        };
        let kinds: Vec<_> = doc.runs(item).iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RunKind::Text, RunKind::HardBreak, RunKind::Text]);
        for dialect in [Dialect::CommonMark, strict] {
            let doc = parse_as(text, dialect);
            assert_eq!(paragraph_runs(&doc)[0][1], ("", RunKind::SoftBreak));
        }
    }

    #[test]
    fn a_note_whose_percents_look_like_code_is_read_once() {
        let obsidian = Dialect::Obsidian { line_breaks: false };
        let parsed = |text: &str, code_pass: bool| {
            CODE_PASS.set(code_pass);
            PASSES.set(0);
            let doc = markdown(text, obsidian, None).map(|d| format!("{d:?}"));
            CODE_PASS.set(false);
            (doc, PASSES.get())
        };
        for (text, passes) in [
            // in code: one pass, its code found by it
            (
                "`printf(\"%%d\")` and\n\n```sh\n${f%%/*}\n```\n\n    %%i\n",
                1,
            ),
            // a `$` price masked before it: the code found by a pass of its own
            ("`%%` and $x$5 then a $\n", 1),
            // one backtick run and no span: a comment after all, parsed again without it
            ("a `b %%hidden%% c\n", 2),
            // a fence line in an HTML block opens no fence
            ("<div>\n```\n</div>\n\n%%hidden%% shown\n", 2),
            // `%%`s that don't look like code: their code found first, one pass
            ("```\n%%\n```\n\n%%\nhidden\n%%\nshown\n", 1),
        ] {
            let (doc, n) = parsed(text, false);
            assert_eq!((&doc, n), (&parsed(text, true).0, passes), "{text:?}");
        }
        assert!(
            !parsed("a `b %%hidden%% c\n", false)
                .0
                .unwrap()
                .contains("hidden")
        );
        parsed("`%%` and $x$5 then a $\n", false);
        assert!(MASK.get().is_some(), "premasked");
        // a host parsed again gives back what its embeds took of the budget
        let embeds = notes(&[("host.md", "other", "embedded text\n")]);
        let host = "a `b %%hidden%%\n\n![[other]]\n";
        let with = |code_pass: bool| {
            CODE_PASS.set(code_pass);
            let doc = parse_with(host, obsidian, Path::new("host.md"), &embeds);
            CODE_PASS.set(false);
            format!("{doc:?}")
        };
        let doc = with(false);
        assert!(doc.contains("embedded text") && !doc.contains("hidden"));
        assert_eq!(doc, with(true));
    }

    /// Random notes of code, `%%`, `$` and blocks parse the same whether their code is found by
    /// their first pass or by a pass of its own (PAR-021). `ADA_PCT_CASES` cases (20,000).
    #[test]
    fn comments_are_the_same_found_either_way() {
        const PARTS: &[&str] = &[
            "%%", "%", "`", "``", "```\n", "~~~\n", "\n", "\n\n", "    ", "> ", "- ", "a", " b ",
            "$", "$$", "5", "\\", "|", "|---|\n", "[[a]]", "<b>", "*", "# ",
        ];
        let cases: usize = (std::env::var("ADA_PCT_CASES").ok())
            .and_then(|n| n.parse().ok())
            .unwrap_or(20_000);
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let obsidian = Dialect::Obsidian { line_breaks: false };
        let (mut looked, mut once) = (0, 0);
        for _ in 0..cases {
            let len = 4 + next() % 40;
            let text: String = (0..len)
                .map(|_| PARTS[(next() % PARTS.len() as u64) as usize])
                .collect();
            if !super::super::comments::any(&text) {
                continue;
            }
            PASSES.set(0);
            let doc = markdown(&text, obsidian, None).map(|d| format!("{d:?}"));
            let passes = PASSES.get();
            CODE_PASS.set(true);
            let reference = markdown(&text, obsidian, None).map(|d| format!("{d:?}"));
            CODE_PASS.set(false);
            assert_eq!(doc, reference, "{text:?}");
            if super::super::comments::look_like_code(&text, 0) {
                looked += 1;
                once += (passes == 1) as usize;
            }
        }
        // the generator reaches both ways
        assert!(
            looked > cases / 50 && once > 0,
            "{looked} looked like code, {once} read once"
        );
    }

    #[test]
    fn obsidians_comments_are_hidden_and_lines_keep_their_numbers() {
        let text = "an %%inline%% comment\n\n%%\n# hidden\n\n- also\n%%\n\nafter `%%code%%`\n\n\
                    ```\n%%\n```\n\n%%unclosed\n\nnever shown\n";
        let doc = parse_as(text, Dialect::Obsidian { line_breaks: false });
        assert_eq!(kinds(&doc.blocks), ["paragraph", "paragraph", "code"]);
        let shown: Vec<_> = (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Paragraph(inl) => Some((doc.plain_text(inl), b.src)),
                _ => None,
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("an comment".to_owned(), LineRange::line(1)),
                ("after %%code%%".to_owned(), LineRange::line(9))
            ]
        );
        assert_eq!(doc.blocks[2].src, LineRange::new(11, 13));
        // CommonMark shows them, their markdown rendered
        let doc = parse(text);
        assert_eq!(
            kinds(&doc.blocks),
            [
                "paragraph",
                "paragraph",
                "heading",
                "list",
                "paragraph",
                "code",
                "paragraph",
                "paragraph"
            ]
        );
    }

    #[test]
    fn highlights_are_found_in_frames_closing_out_of_order() {
        // the inner frame, on line 2, closes before the outer one on line 1
        let mut b = Builder::new("==a==\n==b==\n");
        b.push_inline(InlineKind::Paragraph, LineRange::line(1));
        b.push_text("==a==", InlineStyle::NONE, 1);
        b.push_inline(InlineKind::Paragraph, LineRange::line(2));
        b.push_text("==b==", InlineStyle::NONE, 2);
        b.pop_block();
        b.pop_block();
        let doc = b.finish();
        assert_eq!(styled(&doc, InlineStyle::HIGHLIGHT), ["b", "a"]);
    }

    #[test]
    fn highlights_pair_their_marks_across_markup() {
        let doc = parse(
            "==a *b* [c](u)== and `==x==` x==y==z\n\n\
             a == b == c, x==y, ===no===, ==🔴red== ==open\n\n\
             ==a ==b== c== ==d\ne==\n\n# ==Head==\n",
        );
        let marked = InlineStyle::HIGHLIGHT;
        assert_eq!(
            styled(&doc, marked),
            [
                "a ", "b", " ", "c", "y", "🔴red", "a ", "b", " c", "d", "", "e"
            ]
        );
        let runs = paragraph_runs(&doc);
        assert_eq!(
            runs[2][5..],
            [("", RunKind::SoftBreak), ("e", RunKind::Text)]
        );
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        let emph = doc
            .runs(inl)
            .iter()
            .find(|r| doc.run_text(r) == "b")
            .unwrap();
        assert_eq!(emph.style, InlineStyle::EMPH | marked);
        assert_eq!(doc.plain_text(inl), "a b c and ==x== xyz");
        let BlockKind::Paragraph(inl) = &doc.blocks[1].kind else {
            panic!()
        };
        assert_eq!(
            doc.plain_text(inl),
            "a == b == c, x==y, ===no===, 🔴red ==open"
        );
        assert_eq!(&*doc.anchors[0].text, "Head");
        assert_eq!(&*doc.anchors[0].slug, "head");
    }

    #[test]
    fn inline_math_is_typeset_else_code_as_written() {
        let doc = parse("x $a^2+b_1$ y, $\\RR^n$ and $5 and $10\n");
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(typeset(&doc), ["a² + b₁"]);
        let RunKind::Math(id) = doc.runs(inl)[1].kind else {
            panic!()
        };
        // its TeX is kept, for search
        assert_eq!(doc.math[id as usize].of(&doc.text), "a^2+b_1");
        // TeX past what ada reads is code, as written
        assert_eq!(styled(&doc, InlineStyle::CODE), ["\\RR^n"]);
        // the dollars of prices open no math
        assert_eq!(doc.plain_text(inl), "x a² + b₁ y, \\RR^n and $5 and $10");
    }

    /// Each paragraph's and display math's runs of typeset math, by their text.
    fn typeset(doc: &Doc) -> Vec<&str> {
        (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Paragraph(inl) | BlockKind::Math(inl) => Some(inl),
                _ => None,
            })
            .flat_map(|inl| doc.runs(inl).iter())
            .filter(|r| matches!(r.kind, RunKind::Math(_)))
            .map(|r| doc.run_text(r))
            .collect()
    }

    /// Each paragraph's runs of `style`, by their text.
    fn styled(doc: &Doc, style: InlineStyle) -> Vec<&str> {
        (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Paragraph(inl) => Some(inl),
                _ => None,
            })
            .flat_map(|inl| doc.runs(inl).iter())
            .filter(|r| r.style.contains(style))
            .map(|r| doc.run_text(r))
            .collect()
    }

    #[test]
    fn a_dollar_before_a_digit_closes_no_math() {
        // pandoc's rule: prices in parentheses, after a `~`, around a slash
        let text = "Spend $2.81 effective ($9.53 list), ~$1 or $25/$50.\n";
        let doc = parse(text);
        assert_eq!(styled(&doc, InlineStyle::CODE), Vec::<&str>::new());
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(doc.plain_text(inl), text.trim_end());
        // markup the false math held is read again: the bold closes where it was written
        let doc = parse("**DONE, $1.14 effective** ($11.80 list)\n");
        assert_eq!(
            styled(&doc, InlineStyle::STRONG).concat(),
            "DONE, $1.14 effective"
        );
        // math that closes before a letter or punctuation is still math
        let doc = parse("$x$bar, $y$. and $z$5 more\n");
        assert_eq!(typeset(&doc), ["x", "y"]);
    }

    #[test]
    fn a_dollar_before_a_digit_closes_no_math_across_a_wiki_links_bar() {
        // pulldown-cmark's range for this math starts past the `|`
        let doc = parse("[[a $b|c$9]]\n\n[[:$|x$1]]\n");
        assert_eq!(styled(&doc, InlineStyle::CODE), Vec::<&str>::new());
        assert!(doc.math.is_empty());
        let doc = parse("[[a $b|c$ d]]\n");
        assert_eq!(typeset(&doc), ["b|c"]);
        // its range the closer alone (PAR-014)
        let doc = parse("[[$x|$4 b]]\n");
        assert!(doc.math.is_empty());
        // pulldown-cmark's range for math at a heading's end takes in its tab
        let doc = parse("# $a b`$\t\n");
        assert!(doc.math.is_empty());
    }

    #[test]
    fn a_code_span_opening_on_a_dollar_takes_it_from_math() {
        let doc = parse("about $21 of pod time. `intake.zsh`: default `${@:-1 2}` had\n");
        assert_eq!(styled(&doc, InlineStyle::CODE), ["intake.zsh", "${@:-1 2}"]);
        // a backtick inside math is its own, a prime, and GitHub's $`…`$ is math
        let doc = parse("$f`(x)$ and $b_4` = 0$\n$`\\sqrt{2}`$\n");
        assert_eq!(typeset(&doc), ["f`(x)", "b₄` = 0", "√2"]);
    }

    #[test]
    fn masked_dollars_come_back_in_links_and_html() {
        let doc = parse("$5 in [a $b](x$1) and <i a=\"$2\"> $3\n");
        assert_eq!(styled(&doc, InlineStyle::CODE), Vec::<&str>::new());
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(doc.plain_text(inl), "$5 in a $b and  $3");
        assert_eq!(&*doc.links[0].dest, "x$1");
        assert!(!doc.text.bytes().any(|c| c < 32 && c != b'\n'));
        // a definition's title on the line after it, its `$` masked before the first pass: the
        // reference link, out of its range, gets it back
        let doc = parse("[x]\n\n[x]: /u\n(t $1 a$2)\n");
        assert_eq!(&*doc.links[0].title, "t $1 a$2");
    }

    #[test]
    fn a_false_closer_in_a_label_is_masked_in_its_definition_too() {
        // the math `$1` opened closes on the label's `$`, falsely: masked there and in the
        // definition, the two still match, and the `$1` opens no math
        let doc = parse("$1 [^a$1] b\n\n[^a$1]: c\n");
        assert!(doc.math.is_empty());
        let labels: Vec<_> = doc
            .footnotes
            .iter()
            .map(|f| (&*f.label, f.number))
            .collect();
        assert_eq!(labels, [("a$1", 1)]);
        assert!(doc.footnotes[0].anchor.is_some());
        // every copy: another reference, in another case
        let doc = parse("$1 [^a$1] b [^A$1] c\n\n[^a$1]: c\n");
        assert!(doc.math.is_empty());
        assert_eq!(doc.footnotes.len(), 1);
        let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(
            (doc.runs(inl).iter())
                .filter(|r| matches!(r.kind, RunKind::FootnoteRef(_)))
                .count(),
            2
        );
        // a link reference's, whose definition makes no event
        let doc = parse("$1 [a$1] b [x][a$1]\n\n[a$1]: /u\n");
        assert!(doc.math.is_empty());
        let dests: Vec<_> = doc.links.iter().map(|l| (&*l.dest, l.kind)).collect();
        assert_eq!(
            dests,
            [("/u", LinkKind::Reference), ("/u", LinkKind::Reference)]
        );
        // the copies: every one, in another case, a link's text too; not a copy whose `$` is
        // math's, the first of two in a paragraph opening it
        let copies = |text: &str| {
            let source = text.replacen("$1]", "%1]", 1);
            label_copies(&source, text, 0, &[5], false).0
        };
        let def = "\n\n[b$1]: /u\n";
        assert_eq!(
            copies(&format!("$1 [b$1]\n\n[b$1] x\n\n[B$1](y{def}")),
            [12, 21, 30]
        );
        assert_eq!(copies(&format!("$1 [b$1]\n\n[b$1] and [B$1]{def}")), [29]);
        // a label nothing defines: no copy is a reference
        assert_eq!(copies("$1 [b$1]\n\n[b$1] x\n"), Vec::<usize>::new());
        // one in a definition's title, given back at the links that use it
        let doc = parse("$1 [b$1]\n\n[c]: /u \"[b$1]\"\n\n[c]\n");
        assert_eq!(&*doc.links[0].title, "[b$1]");
        // a reference before a `(` that opens no destination
        let doc = parse("a/$6[a$1]( b\n\n[a$1]: /u\n");
        assert_eq!(doc.links.len(), 1);
    }

    #[test]
    fn a_false_closer_in_a_wiki_links_target_is_masked() {
        // math in a target makes no event: the `$1` closes none, so `$b` reaches the `d$`,
        // and the math crosses the link, as math does
        let doc = parse("[[a $b c$1]] d$\n");
        assert_eq!(
            typeset(&doc).len() + styled(&doc, InlineStyle::CODE).len(),
            1
        );
        assert!(doc.links.is_empty());
        // with no `$` past it, the link stays, its target as written; an embed's too
        let doc = parse("[[a $b c$1]] and ![[e $f g$2|x]]\n");
        assert!(doc.math.is_empty());
        let dests: Vec<_> = doc.links.iter().map(|l| &*l.dest).collect();
        assert_eq!(dests, ["a $b c$1", "e $f g$2"]);
        // math that closes as it should stays the target's
        let doc = parse("[[a $b c$ e]] d$\n");
        assert_eq!(doc.links.len(), 1);
    }

    #[test]
    fn a_pass_masks_no_false_closer_of_math_an_earlier_mask_undoes() {
        // `$x$1` and `$a$2` close falsely: with the `$1` masked, `$x` reaches `y$` and `$a$2`'s
        // math is undone, so its `$2` opens math that closes on `b$`, as pandoc's rule reads
        // it. A pass that masked both lost that math (PAR-016)
        let text = "$x$1 y$a$2 b$\n";
        let (new, _) = parsed(text, Dialect::CommonMark, Masks::Shipped);
        let (old, passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
        let (ideal, ideal_passes) = parsed(text, Dialect::CommonMark, Masks::Ideal);
        assert_eq!((&new, &old), (&ideal, &ideal));
        assert_eq!((passes, ideal_passes), (2, 2));
        // math typeset, or as written where ada can't (`x$1 y`)
        let doc = parse(text);
        assert_eq!(
            typeset(&doc).len() + styled(&doc, InlineStyle::CODE).len(),
            2
        );
        // one whose math, undone, leaves the rest as it was (the next `$` can't close, and no
        // markup was in it) lets the next be masked in the same pass
        let text = "$x$1 y $a$2 b$c\n";
        let (old, passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
        let (ideal, ideal_passes) = parsed(text, Dialect::CommonMark, Masks::Ideal);
        assert_eq!(old, ideal);
        assert_eq!((passes, ideal_passes), (2, 3));
        // not past markup the undone math held: undone, `$x`'s math frees a backtick, whose code
        // span takes `$a$2`
        let text = "$x `y$1 z $a$2 b` c$\n";
        let (old, passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
        assert_eq!(old, parsed(text, Dialect::CommonMark, Masks::Ideal).0);
        assert_eq!(passes, 2);
        assert_eq!(styled(&parse(text), InlineStyle::CODE), ["y$1 z $a$2 b"]);
        // paragraphs are read apart: each one's first false closer is masked in the same pass
        let text = "$x$1 y $a$2 b$c\n\n$x$1 y$a$2 b$\n";
        let (old, passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
        let (ideal, ideal_passes) = parsed(text, Dialect::CommonMark, Masks::Ideal);
        assert_eq!(old, ideal);
        assert_eq!((passes, ideal_passes), (2, 4));
    }

    #[test]
    fn a_wiki_links_targets_false_closers_are_masked_in_one_pass() {
        // `c$1` closes math in its target falsely: masked, `$b`'s math is undone (the next `$`
        // can't close), and the link reads back over its target as before, so the `d$2` past
        // it, and the `e$3` past that, are masked in the same pass (PAR-020; one pass a target
        // before, as the reference takes)
        for (text, ideal_passes) in [
            ("$1 [[a $b c$1]] and $2 [[d$2]]\n", 3),
            ("$1 [[a $b c$1]] and $2 [[d$2]] and $3 [[e$3]]\n", 4),
        ] {
            let (new, passes) = parsed(text, Dialect::CommonMark, Masks::Shipped);
            let (old, old_passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
            let (ideal, n) = parsed(text, Dialect::CommonMark, Masks::Ideal);
            assert_eq!((&new, &old), (&ideal, &ideal), "{text:?}");
            assert_eq!((passes, old_passes, n), (2, 2, ideal_passes), "{text:?}");
        }
        // no `$` closes math: both links stay, their targets as written
        let doc = parse("$1 [[a $b c$1]] and $2 [[d$2]]\n");
        let dests: Vec<_> = doc.links.iter().map(|l| &*l.dest).collect();
        assert_eq!(dests, ["a $b c$1", "d$2"]);
        assert!(doc.math.is_empty());
    }

    #[test]
    fn a_labels_copies_are_masked_as_pandocs_rule_meets_the_false_closers() {
        // a pass masks two contexts' false closers (the `$`s of `[A$1]<` and `$6)`): with both
        // masked, the copies past `@x` and `=` are in math, with the first alone (the
        // reference's first pass) out of it, and masked. Masked one at a time, the copies are
        // the reference's (PAR-019)
        let first = "[a$1]:> [A$1]<$7``a$2**~\n\n$a9/$6)`$[[$x|@x[A$1]$$`=[a$1]:- |] \
                     $[^a$1]:1[^a$1]:";
        // a copy masked undoes math another copy was in (`$1]($1[^a$1]` with the last `$`
        // masked): judged again in the same pass, as the next meets no false closer to do it
        let again = "`$\t# ;<%\n   [A$1]<|```\n$<{$$1```\n.[[$x|~2\t$4,[[[a $b#- [^a$1];\n\
                     [^a$1]:x[^a$1]($1[^a$1]<a b=\"";
        for (text, dialect) in [
            (first, Dialect::Obsidian { line_breaks: false }),
            (again, Dialect::CommonMark),
        ] {
            let (new, _) = parsed(text, dialect, Masks::Shipped);
            let (old, _) = parsed(text, dialect, Masks::PassesOnly);
            let (ideal, _) = parsed(text, dialect, Masks::Ideal);
            assert_eq!((&new, &old), (&ideal, &ideal), "{text:?}");
        }
    }

    /// `text` parsed as [`markdown`] does with `masks`: its doc's `Debug` ([`merged`]) and the
    /// passes it took.
    fn parsed(text: &str, dialect: Dialect, masks: Masks) -> (Option<String>, usize) {
        MASKS.set(masks);
        PASSES.set(0);
        MASK.set(None);
        KEPT.set(0);
        CAPPED.set(false);
        let doc = markdown(text, dialect, None);
        MASKS.set(Masks::Shipped);
        // no mask is left in the doc: every string holding one got its `$`s back (a control
        // byte as `Debug` escapes it; punctuation inside `Debug`'s strings, which writes some
        // of it itself)
        if let (Some(d), Some(m)) = (&doc, MASK.get()) {
            let debug = format!("{d:?}");
            let left = match m.is_ascii_control() {
                true => {
                    let m = format!("{:?}", char::from(m));
                    debug.contains(&m[1..m.len() - 1])
                }
                false => in_strings(&debug, char::from(m)),
            };
            assert!(!left, "{text:?}: {:?} left in the doc", char::from(m));
        }
        (doc.map(|d| merged(&d)), PASSES.get())
    }

    /// Whether `m` is inside a string of `debug`, a `Debug` output, but an anchor's slug, whose
    /// `-`s are the slugger's own.
    fn in_strings(debug: &str, m: char) -> bool {
        let (mut inside, mut escape) = (false, false);
        let mut slug = false;
        for (i, c) in debug.char_indices() {
            if !inside && c == '"' {
                slug = debug[..i].ends_with("slug: ");
            }
            if slug && inside && !escape && c == m {
                continue;
            }
            match (inside, escape, c) {
                (true, true, _) => escape = false,
                (true, false, '\\') => escape = true,
                (true, false, '"') | (false, _, '"') => inside = !inside,
                (true, false, c) if c == m => return true,
                _ => {}
            }
        }
        false
    }

    /// `doc`'s `Debug` with each text run merged into the one before it where only pulldown-cmark
    /// split them: a masked `$` is text of one event with its neighbors', a `$` one of its own.
    fn merged(doc: &Doc) -> String {
        let mut s = format!("{doc:?}");
        // the runs, unmerged: given merged below
        let i = s.find(", runs: [").unwrap();
        let j = i + s[i..].find(']').unwrap();
        s.replace_range(i..=j, "");
        let mut bounds = std::collections::HashSet::new();
        let slots = |s: &str| -> Vec<(usize, u32, u32, usize)> {
            // runs' slots only: a table row's are its cells'
            let key = "runs: Slots { start: ";
            let mut out = Vec::new();
            let mut at = 0;
            while let Some(i) = s[at..].find(key).map(|i| i + at) {
                let rest = &s[i + key.len()..];
                let comma = rest.find(',').unwrap();
                let start = rest[..comma].parse().unwrap();
                let rest2 = &rest[comma + ", end: ".len()..];
                let close = rest2.find(' ').unwrap();
                let end = rest2[..close].parse().unwrap();
                let stop = i + key.len() + comma + ", end: ".len() + close + " }".len();
                out.push((i, start, end, stop));
                at = stop;
            }
            out
        };
        for &(_, a, b, _) in &slots(&s) {
            bounds.extend([a, b]);
        }
        // a run starts a merged one, or goes on the one before
        let mut starts = Vec::with_capacity(doc.runs.len());
        let mut runs: Vec<Run> = Vec::new();
        for (i, r) in doc.runs.iter().enumerate() {
            let goes_on = runs.last().is_some_and(|p| {
                !bounds.contains(&(i as u32))
                    && matches!((p.kind, r.kind), (RunKind::Text, RunKind::Text))
                    && (p.style, p.link, p.src_line, p.text.end)
                        == (r.style, r.link, r.src_line, r.text.start)
            });
            match runs.last_mut() {
                Some(p) if goes_on => p.text.end = r.text.end,
                _ => runs.push(*r),
            }
            starts.push(u32::from(!goes_on));
        }
        let new_index = |x: u32| starts[..x as usize].iter().sum::<u32>();
        let mut out = String::new();
        let mut at = 0;
        for (i, a, b, stop) in slots(&s) {
            out.push_str(&s[at..i]);
            out.push_str(&format!(
                "runs: Slots {{ start: {}, end: {} }}",
                new_index(a),
                new_index(b)
            ));
            at = stop;
        }
        out.push_str(&s[at..]);
        format!("{out}\nmerged: {runs:?}")
    }

    #[test]
    fn prices_parse_once() {
        for text in [
            "Spend $2.81 effective ($9.53 list), ~$1 or $25/$50.\n",
            "**DONE, $1.14 effective** ($11.80 list)\n",
            "$5 in [a $b](x$1) and <i a=\"$2\"> $3\n",
            // the second closes nothing: a blank before it
            "about $1 (or $2) and\n\n$3 ($4.\n",
            // a fence's info string gets its back; a footnote's label keeps its
            "$1 a\n```x$5 ($6\nb\n```\n\n$1 [^a] b ($2\n\n[^a]: b ($2\n",
            // a code span opening on a `$`
            "about $21 of pod time. `intake.zsh`: default `${@:-1 2}` had\n",
        ] {
            let (new, passes) = parsed(text, Dialect::CommonMark, Masks::Shipped);
            let (old, old_passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
            assert_eq!(new, old, "{text:?}");
            assert_eq!(
                new,
                parsed(text, Dialect::CommonMark, Masks::Ideal).0,
                "{text:?}"
            );
            assert!(
                passes == 1 && old_passes > 1,
                "{text:?}: {passes}, {old_passes}"
            );
        }
    }

    #[test]
    fn a_dollar_that_may_open_math_is_left_to_the_passes() {
        // each after a `$` that can open: one no math can close on is left as it is
        let never = |t: &str| -> Vec<usize> {
            let at = never_opens(&format!("$a {t}"), 0);
            at.into_iter().map(|c| c - 3).collect()
        };
        assert_eq!(never_opens("($5 and $6)\n", 0), Vec::<usize>::new());
        assert_eq!(never("($5 and $6)\n"), [1]);
        // the next `$` closes: math
        assert_eq!(never("x$1+1$ y\n"), Vec::<usize>::new());
        // balanced braces between move no context; a `$` in them, or a `}` past its, does
        assert_eq!(never("x$1 {a,{b}} $2\n"), [1]);
        assert_eq!(never("x$1 {a $b} $2\n"), Vec::<usize>::new());
        assert_eq!(never("x$1 a} $2\n"), Vec::<usize>::new());
        // a `$$` before it in its paragraph, not past a blank line
        assert_eq!(never("$$ a ($5 b $$\n"), Vec::<usize>::new());
        assert_eq!(never("$$ a $$\n\n$b ($5 b\n"), [13]);
        // escaped, or after a blank, or after a backslash's escaped `$`
        assert_eq!(never("x\\$1 $2 ($3\n"), [9]);
        assert_eq!(never("a \\\\$1 b\n"), [4]);
        // the next `$` past a blank line is another paragraph's
        assert_eq!(never("x$1 a\n  \nb$2\n"), [1]);
        assert_eq!(never("x$1 a\n  \n$c b$2\n"), [1, 13]);
        // a backtick's: a code span opening on a `$`, unless math opened on a backtick before it
        assert_eq!(never("$5 a `${b}` c\n"), [6]);
        assert_eq!(never("$`a` `$b`\n"), Vec::<usize>::new());
        assert_eq!(never("$`a`\n\n$c `$b` c\n"), [10]);
        // between brackets on its line: a label nothing defines as any other (PAR-017); not a
        // bracket on one side only, nor a wiki link's target
        assert_eq!(never("[^a$1] [(x$2) ]\n[[b\n(c$3]]\nx] ($4\n"), [3, 10, 31]);
        assert_eq!(never("[[c$1]] b\n"), Vec::<usize>::new());
        // a defined label's `$` in every copy, math reaching it or not, or in none: the
        // definition's is masked too; one copy opening math keeps all of them unmasked
        assert_eq!(never("[^a$1] b\n\n[^a$1]: c\n"), [3, 13]);
        assert_eq!(
            never("[^a$1] b\n\n[^A$1]: c\n\nd [^a$1] e$\n"),
            Vec::<usize>::new()
        );
        assert_eq!(
            never("[^a$1] b\n\nd `[^a$1]` e\n\n[^a$1]: c\n"),
            [3, 16, 27]
        );
    }

    #[test]
    fn a_labels_false_closer_is_masked_before_the_first_pass() {
        // each `$` a false closer the passes met one at a time (PAR-017), masked up front in
        // each copy of its label, so the note parses once, as the passes and pandoc's rule read it
        for text in [
            "Spend $1 [a$1] and $2 [b$2] and $3 [c$3] and $4 [d$4].\n",
            "Spend $1 [^a$1] and $2 [^b$2] and $3 [x$3] and $4 [^a$1].\n\n[^a$1]: b\n[^b$2]: c\n",
            "Spend $1 [A$1] and $2 [b](/u).\n\n[a$1]: /v\n",
        ] {
            let (new, passes) = parsed(text, Dialect::CommonMark, Masks::Shipped);
            let (old, old_passes) = parsed(text, Dialect::CommonMark, Masks::PassesOnly);
            assert_eq!(new, old, "{text:?}");
            assert_eq!(
                new,
                parsed(text, Dialect::CommonMark, Masks::Ideal).0,
                "{text:?}"
            );
            assert!(
                passes == 1 && old_passes > 1,
                "{text:?}: {passes}, {old_passes}"
            );
        }
        // the references still match their definitions
        let doc = parse("Spend $1 [^a$1] b.\n\n[^a$1]: c\n");
        assert!(doc.footnotes[0].anchor.is_some());
        // a copy a control byte can't take (inside `<…>`): the label left to the passes, which
        // keep its false closer
        CONTROL.set(true);
        let (_, passes) = parsed(
            "$y [^a$1]\n\n<[^a$1]>\n\n[^a$1]: b\n",
            Dialect::CommonMark,
            Masks::Shipped,
        );
        CONTROL.set(false);
        assert_eq!((passes, NEVER.take().len()), (2, 3));
    }

    #[test]
    fn a_control_byte_masks_no_dollar_it_would_change_the_parse_at() {
        let safe = |t: &str| -> Vec<usize> {
            let t = format!("$a {t}");
            let mut never = never_opens(&t, 0);
            line_safe(t.as_bytes(), &mut never, true, true);
            never.into_iter().map(|c| c - 3).collect()
        };
        // flanking, a destination (on the next line too), a reference's line and the next, `<…>`
        assert_eq!(safe("a**$1** _$2_ c$3\n"), [9, 14]);
        assert_eq!(safe("`$_a` `$b`\n"), [7]);
        assert_eq!(safe("[a](x$1) [b](\ny$2) z$3\n"), [20]);
        assert_eq!(safe("[a]: x$1\n[b]:\n y$2\nz$3\n"), [20]);
        assert_eq!(safe("<x$1> <y\nz$2\n"), [10]);
        // a reference's line under any mask: its destination is out of its links' ranges
        let mut never = never_opens("$a [b]: x$1\n", 0);
        line_safe(b"$a [b]: x$1\n", &mut never, false, true);
        assert_eq!(never, Vec::<usize>::new());
    }

    #[test]
    fn a_wiki_links_events_out_of_order_get_their_dollars_back() {
        // pulldown-cmark gives the inner image's start (at the `](`) before the math at the
        // `$6`: `parsed` asserts no mask is left in the doc
        let text = "$$1$$(!)![[![^a$1]/$6<a b=\"[[$x|](\">1. )%[a$1]> \
                    http://http://$http://c$1]]]][A$1][^a$1]\t===";
        parsed(text, Dialect::CommonMark, Masks::Shipped);
        let text = "\n\n\n\na($1a {[[a $b/`$--><http://[a$1]:{[^a$1]|>$#*$-->+c$1]]][@b.c>\
                    </[[a $b@\">#  :```\n$ac$1]]";
        FORCE.set(Some(b'%'));
        parsed(text, Dialect::CommonMark, Masks::Shipped);
        FORCE.set(None);
        // the image and the link overlap: the image's text past the link's end comes last
        let text = "b[A$1];,b<$7})]]](\n\">b$![[[a$1]:[a$1]http://    $a](]()9[^a$1]$4c$1]]`";
        parsed(text, Dialect::CommonMark, Masks::Shipped);
    }

    #[test]
    fn an_entity_of_the_mask_byte_stays_itself() {
        // pulldown-cmark decodes `&#37;` and `&percnt;` to `%` in the destination it masked a
        // `$` in: the mask is a byte the text doesn't write either way
        for text in ["$a [b](/u&#37;x$1) c\n", "$a [b](/u&percnt;x$1) c\n"] {
            let doc = parse(text);
            assert_eq!(&*doc.links[0].dest, "/u%x$1", "{text:?}");
        }
    }

    #[test]
    fn a_control_byte_keeps_a_false_closer_it_would_change_the_parse_at() {
        let control = |text: &str| {
            CONTROL.set(true);
            KEPT.set(0);
            let doc = parse(text);
            CONTROL.set(false);
            (doc, KEPT.get())
        };
        let struck = |doc: &Doc| {
            let BlockKind::Paragraph(inl) = &doc.blocks[0].kind else {
                panic!()
            };
            (doc.runs(inl).iter()).any(|r| r.style.contains(InlineStyle::STRIKE))
        };
        // a control byte after `~` lets it open a strikethrough the `$` didn't: the false
        // closer is kept, its math read as pulldown-cmark reads it (PAR-015)
        let text = "$x a~$3 b~\n";
        let (doc, kept) = control(text);
        assert_eq!((doc.math.len(), struck(&doc), kept), (1, false, 1));
        let doc = parse(text);
        assert_eq!((doc.math.len(), struck(&doc)), (0, false));
        // a label's `$` a copy can't take a control byte at (inside `<…>`): unmasked in every
        // copy, the false closer kept, so the copy the math leaves still matches its definition
        let (doc, kept) = control("$y [^a$1]\n\n<[^a$1]>\n\n[^a$1]: b\n");
        assert_eq!((doc.math.len(), kept), (1, 1));
        let BlockKind::Paragraph(inl) = &doc.blocks[1].kind else {
            panic!()
        };
        assert_eq!(
            (doc.runs(inl).iter())
                .filter(|r| matches!(r.kind, RunKind::FootnoteRef(_)))
                .count(),
            1
        );
        assert!(doc.footnotes[0].anchor.is_some());
    }

    /// Texts of markdown's bytes around `$`, random but the same each run: a parse reads the
    /// math as pandoc's rule does ([`Masks::Ideal`]), with the masks before the first pass and
    /// with the passes alone, however many passes it takes (PAR-016), a label's copies masked
    /// as the reference masks them (PAR-019). But past the last pass ([`MAX_MATH_PASSES`]). Not
    /// a text that leaves the reference no punctuation to mask with. A third mask with a
    /// control byte ([`CONTROL`]), whose passes read as punctuation's, and as the reference,
    /// wherever they keep no false closer (PAR-015). Labels and their definitions, and wiki links' targets, hold
    /// false closers too (PAR-014). `ADA_PREMASK_CASES` sets how many (50,000).
    #[test]
    fn masks_before_the_first_pass_read_math_as_pandoc_does() {
        const PARTS: &[&str] = &[
            "$", "$", "$", "$", "$$", "1", "2", "9", "a", "x", "b", " ", " ", " ", "\n", "\n\n",
            "{", "}", "`", "``", "\\", "(", ")", "<", ">", "*", "**", "_", "[", "]", "](", "[[",
            "]]", "|", "- ", "> ", "1. ", "# ", "%", "?", "@", ".", ",", "#", "&", ";", ":", "!",
            "=", "==", "~", "\t", "    ", "```\n", "[^1]", "<a b=\"", "\">", "http://", "a@b.c",
            // prices and math
            "($1", "a$2", "~$3", "$4", "x$5 ", "/$6", "$a", "b$", "$`", "`$", "$$1", "<$7", "{$8",
            // labels and definitions (after a "\n\n"), wiki links' targets
            "[^a$1]", "[a$1]", "[A$1]", "[^a$1]:", "[a$1]:", "[[a $b", "c$1]]", "[[$x|", "![[",
        ];
        let cases: usize = (std::env::var("ADA_PREMASK_CASES").ok())
            .and_then(|n| n.parse().ok())
            .unwrap_or(50_000);
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (mut once, mut premasked, mut controlled, mut old_once, mut kept) = (0, 0, 0, 0, 0);
        let (mut again, mut capped_n) = (0, 0);
        for _ in 0..cases {
            let len = 4 + next() % 40;
            let text: String = (0..len)
                .map(|_| PARTS[(next() % PARTS.len() as u64) as usize])
                .collect();
            let dialect = match next() % 4 {
                0 => Dialect::Obsidian { line_breaks: false },
                _ => Dialect::CommonMark,
            };
            // the reference masks with punctuation: a control byte flanks otherwise
            if unused_byte(&text).is_none_or(|m| m.is_ascii_control()) {
                continue;
            }
            let control = next() % 3 == 0;
            CONTROL.set(control);
            let (new, passes) = parsed(&text, dialect, Masks::Shipped);
            CONTROL.set(false);
            let (capped, keeps) = (CAPPED.get(), KEPT.get() > 0);
            let masked = !NEVER.take().is_empty();
            // a control byte's passes read as punctuation's, but where a false closer is kept
            // (PAR-015)
            if control {
                match keeps {
                    false => {
                        let (punct, _) = parsed(&text, dialect, Masks::Shipped);
                        if !capped && !CAPPED.get() {
                            assert_eq!(new, punct, "{text:?}, a control byte against punctuation");
                        }
                    }
                    true => kept += 1,
                }
            }
            let (ideal, _) = parsed(&text, dialect, Masks::Ideal);
            if passes == 1 {
                once += 1;
                premasked += usize::from(masked);
                controlled += usize::from(masked && control);
            }
            // the passes' masks are pandoc's, however many passes they take (PAR-016), a
            // label's copies with them (PAR-019), but past the last, and a control byte's where
            // it keeps a false closer
            if capped {
                capped_n += 1;
            } else if !keeps {
                again += usize::from(passes > 1);
                assert_eq!(new, ideal, "{text:?}, {passes} passes");
            }
            let (old, old_passes) = parsed(&text, dialect, Masks::PassesOnly);
            old_once += usize::from(old_passes == 1);
            if !CAPPED.get() {
                assert_eq!(
                    old, ideal,
                    "{text:?}, the passes alone, {old_passes} passes"
                );
            }
        }
        let counts = format!(
            "{once} parsed once, {premasked} with masks ({controlled} a control byte's), \
             {old_once} with the passes alone, {kept} keeping a false closer, {again} parsed \
             again as pandoc's rule reads them, {capped_n} past the last pass"
        );
        eprintln!("{counts}");
        // labels' and targets' parts send more texts through passes (PAR-014), and the prices
        // after a wiki link holding a `$` (PAR-016): a quarter once, 3.6% with masks, 0.9% with
        // a control byte's
        assert!(
            once > cases / 5 && premasked > cases / 40 && controlled > cases / 200,
            "{counts}"
        );
    }

    /// The mask bytes past the first five ([`rarer_byte`]) read as `%` does, which means nothing
    /// anywhere: random texts of markdown's bytes around `$` (those of
    /// [`masks_before_the_first_pass_read_math_as_pandoc_does`], with entities, HTML comments,
    /// autolinks and the bytes themselves), each parsed with each byte that it lacks and that
    /// [`rarer_byte`] would take for it, against `%` (PAR-018). `-`, which isn't taken, doesn't:
    /// `` `$-> `` ends an HTML comment. `ADA_PREMASK_CASES` sets how many (50,000).
    #[test]
    fn rarer_mask_bytes_read_as_percent_does() {
        const PARTS: &[&str] = &[
            "$", "$", "$", "$", "$$", "1", "2", "9", "a", "x", " ", " ", "\n", "\n\n", "{", "}",
            "`", "\\", "(", ")", "<", ">", "*", "_", "[", "]", "](", "[[", "]]", "|", "- ", "> ",
            "1. ", "# ", "%", "@", ".", "#", "&", ";", ":", "!", "=", "~", "    ", "```\n",
            "<a b=\"", "\">", "http://", "a@b.c", "+", "/", "<!--", "-->", "&amp", "&#3", "&#x",
            "<x:", "<a@", "@b.c>", "</", "/>", "`$-> ", "($1", "a$2", "~$3", "x$5 ", "$a", "b$",
            "$`", "`$", "[^a$1]", "[a$1]:", "[[a $b", "c$1]]",
        ];
        let percent = |text: &str, dialect: Dialect, m: u8| {
            FORCE.set(Some(m));
            let doc = parsed(text, dialect, Masks::Shipped).0;
            FORCE.set(None);
            doc
        };
        let cases: usize = (std::env::var("ADA_PREMASK_CASES").ok())
            .and_then(|n| n.parse().ok())
            .unwrap_or(50_000);
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut compared = [0usize; 6];
        for _ in 0..cases {
            let len = 4 + next() % 40;
            let text: String = (0..len)
                .map(|_| PARTS[(next() % PARTS.len() as u64) as usize])
                .collect();
            let dialect = match next() % 4 {
                0 => Dialect::Obsidian { line_breaks: false },
                _ => Dialect::CommonMark,
            };
            let b = text.as_bytes();
            let entities = Entities::new(b);
            let free = |c: u8| !b.contains(&c) && !entities.writes(c);
            if !free(b'%') {
                continue;
            }
            let reference = percent(&text, dialect, b'%');
            for (i, (c, ok)) in rarer_bytes(&maskable_spots(b)).into_iter().enumerate() {
                if ok && free(c) {
                    compared[i] += 1;
                    assert_eq!(
                        percent(&text, dialect, c),
                        reference,
                        "{text:?}: {:?} against `%`",
                        char::from(c)
                    );
                }
            }
        }
        eprintln!("compared, `/ # - + . ;`: {compared:?}");
        assert!(compared.iter().all(|&n| n > cases / 20), "{compared:?}");
        // each condition keeps a byte from a parse it would change: `#` after an `&` starts a
        // numeric entity, `;` after an entity's name ends it, and inside `<…>` the others make
        // a tag (`<a-1>`) or an autolink of what wasn't one, or `;` breaks an email autolink
        for (text, c) in [
            ("$a &$35; b\n", b'#'),
            ("$a &amp$1 b\n", b';'),
            ("$a <a$1@b.c> b\n", b';'),
            ("$a <a$1> b\n", b'-'),
            ("$a <ab$1:x> b\n", b'+'),
            ("$a <a@b$1c> b\n", b'.'),
        ] {
            assert_ne!(
                percent(text, Dialect::CommonMark, c),
                percent(text, Dialect::CommonMark, b'%'),
                "{text:?}: {:?}",
                char::from(c)
            );
        }
    }

    /// The `$`s masked before the first pass in the file at `ADA_FILE`, each with its context:
    /// `cargo test --release premask_file -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads a file"]
    fn premask_file() {
        let text = std::fs::read_to_string(std::env::var("ADA_FILE").expect("ADA_FILE")).unwrap();
        let mut source = Cow::Borrowed(&*text);
        let mut never = never_opens(&text, 0);
        let all = never.len();
        let mask = premasked(&mut source, &text, 0, &mut never);
        eprintln!("{all} found, {} masked with {mask:?}", never.len());
        for at in never {
            let from = text.floor_char_boundary(at.saturating_sub(50));
            let to = text.ceil_char_boundary((at + 20).min(text.len()));
            eprintln!("  {at}: {:?}", &text[from..to]);
        }
    }

    /// The vault at `ADA_VAULT` parsed both ways, as
    /// [`masks_before_the_first_pass_read_math_as_pandoc_does`]: the notes parsed more
    /// than once each way, and those that differ from the passes alone or pandoc's rule (each to
    /// be read: a label's copies masked at another pass, or a fault): `cargo test --release
    /// premask_vault -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads a vault"]
    fn premask_vault() {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
                let path = entry.path();
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => walk(&path, out),
                    Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "md") => {
                        out.push(path);
                    }
                    _ => {}
                }
            }
        }
        let root = std::env::var("ADA_VAULT").expect("ADA_VAULT");
        let mut paths = Vec::new();
        walk(Path::new(&root), &mut paths);
        paths.sort();
        let (mut again, mut old_again, mut differ, mut off, mut kept) = (0, 0, 0, 0, 0);
        for path in &paths {
            let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
            for dialect in [Dialect::CommonMark, Dialect::Obsidian { line_breaks: true }] {
                let (new, passes) = parsed(&text, dialect, Masks::Shipped);
                if KEPT.get() > 0 {
                    kept += 1;
                    eprintln!("keeps {} false closers: {}", KEPT.get(), path.display());
                }
                let (old, old_passes) = parsed(&text, dialect, Masks::PassesOnly);
                let (ideal, _) = parsed(&text, dialect, Masks::Ideal);
                again += usize::from(passes > 1);
                old_again += usize::from(old_passes > 1);
                if new != old {
                    differ += 1;
                    eprintln!(
                        "differs from the passes alone: {} ({dialect:?})",
                        path.display()
                    );
                }
                if new != ideal {
                    off += 1;
                    eprintln!(
                        "differs from pandoc's rule: {} ({dialect:?})",
                        path.display()
                    );
                    // `ADA_DUMP=DIR`: each side's doc to DIR/NOTE.{new,ideal,old}
                    if let Ok(dir) = std::env::var("ADA_DUMP") {
                        let name = path.file_stem().unwrap().to_string_lossy();
                        for (side, doc) in [("new", &new), ("ideal", &ideal), ("old", &old)] {
                            let out = Path::new(&dir).join(format!("{name}.{side}"));
                            std::fs::write(out, doc.as_deref().unwrap_or("")).unwrap();
                        }
                    }
                }
                if passes > 1 && matches!(dialect, Dialect::CommonMark) {
                    eprintln!("{passes} passes ({old_passes} alone): {}", path.display());
                    WRONG.take();
                    parsed(&text, dialect, Masks::Shipped);
                    for at in WRONG.take() {
                        let line = text[..at].matches('\n').count() + 1;
                        let from = text.floor_char_boundary(at.saturating_sub(60));
                        let to = text.ceil_char_boundary((at + 20).min(text.len()));
                        eprintln!("  line {line}: {:?}", &text[from..to]);
                    }
                }
            }
        }
        eprintln!(
            "{} notes, both dialects: {again} parsed again ({old_again} with the passes alone), \
             {differ} differ from the passes alone, {off} from pandoc's rule, {kept} keep a false \
             closer",
            paths.len()
        );
        assert_eq!(off, 0);
    }

    /// Typeset display math: each row and its source line.
    fn math_rows<'d>(doc: &'d Doc, b: &Block) -> Vec<(&'d str, u32)> {
        let BlockKind::Math(inl) = &b.kind else {
            panic!("not typeset math: {:?}", b.kind);
        };
        (doc.runs(inl).iter())
            .filter(|r| matches!(r.kind, RunKind::Math(_)))
            .map(|r| (doc.run_text(r), r.src_line))
            .collect()
    }

    fn math(b: &Block) -> (&str, u32, LineRange) {
        match &b.kind {
            BlockKind::Code(c) if c.lang.as_deref() == Some("math") => {
                (&c.text, c.first_line, b.src)
            }
            other => panic!("not math: {other:?}"),
        }
    }

    #[test]
    fn display_math_is_a_block_of_its_rows() {
        let text = "before\n$$\n\\begin{aligned}\na &= 1 \\\\\nbb &= \\frac{x}{2}\n\\end{aligned}\n$$\nafter\n";
        let doc = parse(text);
        assert_eq!(kinds(&doc.blocks), ["paragraph", "math", "paragraph"]);
        assert_eq!(doc.blocks[1].src, LineRange::new(2, 7));
        // aligned on their `&`, each row on its line
        assert_eq!(
            math_rows(&doc, &doc.blocks[1]),
            [("\u{a0}a = 1", 4), ("bb = x/2", 5)]
        );
        // a matrix's rows share its line's TeX
        let doc = parse("$$\nA = \\begin{pmatrix} 1 \\\\ 2 \\end{pmatrix}\n$$\n");
        assert_eq!(
            math_rows(&doc, &doc.blocks[0]),
            [("A = ⎛1⎞", 2), ("\u{a0}\u{a0}\u{a0}\u{a0}⎝2⎠", 2)]
        );
        assert_eq!(doc.math.len(), 1);
    }

    #[test]
    fn display_math_past_what_ada_reads_is_a_block_of_its_lines() {
        let text = "before\n$$\n\\sum_i \\RR_i\n= 1\n$$\nafter\n\n$$x = \\RR$$\n";
        let doc = parse(text);
        assert_eq!(
            kinds(&doc.blocks),
            ["paragraph", "code", "paragraph", "code"]
        );
        assert_eq!(doc.blocks[0].src, LineRange::line(1));
        assert_eq!(
            math(&doc.blocks[1]),
            ("\\sum_i \\RR_i\n= 1", 3, LineRange::new(2, 5))
        );
        assert_eq!(doc.blocks[2].src, LineRange::line(6));
        // the paragraphs split around it hold no break where the math was
        assert_eq!(
            paragraph_runs(&doc),
            [
                vec![("before", RunKind::Text)],
                vec![("after", RunKind::Text)]
            ]
        );
        assert_eq!(math(&doc.blocks[3]), ("x = \\RR", 8, LineRange::line(8)));
    }

    #[test]
    fn display_math_where_no_block_goes_is_inline() {
        let doc = parse("# A $$x$$ b\n\n| h |\n|---|\n| $$z$$ |\n");
        assert_eq!(kinds(&doc.blocks), ["heading", "table"]);
        let BlockKind::Heading { inlines, .. } = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(doc.plain_text(inlines), "A x b");
        // rows don't go on a line: the TeX as code
        let doc = parse("# A $$x \\\\ y^2$$ b\n");
        assert_eq!(styled_in_headings(&doc), ["x \\\\ y^2"]);
    }

    fn styled_in_headings(doc: &Doc) -> Vec<&str> {
        (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { inlines, .. } => Some(inlines),
                _ => None,
            })
            .flat_map(|inl| doc.runs(inl).iter())
            .filter(|r| r.style.contains(InlineStyle::CODE))
            .map(|r| doc.run_text(r))
            .collect()
    }

    #[test]
    fn display_math_in_a_list_item_and_a_quote() {
        let doc = parse("- a\n  $$\n  x\n  $$\n\n> $$\n> y\n> $$\n");
        let BlockKind::List(list) = &doc.blocks[0].kind else {
            panic!()
        };
        assert_eq!(kinds(&list.items[0].blocks), ["paragraph", "math"]);
        assert_eq!(math_rows(&doc, &list.items[0].blocks[1]), [("x", 3)]);
        let BlockKind::Quote { blocks, .. } = &doc.blocks[1].kind else {
            panic!()
        };
        assert_eq!(kinds(blocks), ["math"]);
        assert_eq!(math_rows(&doc, &blocks[0]), [("y", 7)]);
    }

    #[test]
    fn note_embeds_are_links() {
        let doc =
            parse("![[Other#Heading]] and ![[Other|Alias]]\n\n![[file.pdf]]\n\n![x](note.md)\n");
        let embeds: Vec<_> = paragraph_runs(&doc)
            .concat()
            .into_iter()
            .filter(|(_, k)| matches!(k, RunKind::Embed(_)))
            .map(|(t, _)| t)
            .collect();
        assert_eq!(embeds, ["Other#Heading", "Alias", "file.pdf", "x"]);
        let kinds: Vec<_> = doc.links.iter().map(|l| (&*l.dest, l.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("Other#Heading", LinkKind::Wiki { aliased: false }),
                ("Other", LinkKind::Wiki { aliased: true }),
                ("file.pdf", LinkKind::Wiki { aliased: false }),
                ("note.md", LinkKind::Inline),
            ]
        );
    }

    /// Notes for [`parse_with`], each embedded from `host.md` by its name as a wiki link.
    fn notes(notes: &[(&str, &str, &str)]) -> Embeds {
        let notes = notes.iter().map(|&(from, name, text)| embed::Note {
            from: from.into(),
            target: crate::links::LocalTarget::Wiki(name.into()),
            path: format!("{name}.md").into(),
            mtime: Some(1),
            text: text.into(),
        });
        Embeds(notes.collect::<Vec<_>>().into())
    }

    fn transcluded(text: &str, embeds: &Embeds) -> Doc {
        parse_with(text, Dialect::CommonMark, Path::new("host.md"), embeds)
    }

    fn embedded(b: &Block) -> &Embed {
        match &b.kind {
            BlockKind::Embed(e) => e,
            _ => panic!("not an embed: {}", kinds(std::slice::from_ref(b))[0]),
        }
    }

    /// Every search unit, table and code block a document numbers, wherever its blocks are.
    fn numbered(doc: &Doc) -> (Vec<UnitId>, Vec<TableId>, Vec<CodeId>) {
        fn walk(blocks: &[Block], n: &mut (Vec<UnitId>, Vec<TableId>, Vec<CodeId>)) {
            for b in blocks {
                match &b.kind {
                    BlockKind::Heading { inlines, .. }
                    | BlockKind::Paragraph(inlines)
                    | BlockKind::Math(inlines) => n.0.push(inlines.unit),
                    BlockKind::List(l) => l.items.iter().for_each(|i| walk(&i.blocks, n)),
                    BlockKind::Quote { callout, blocks } => {
                        n.0.extend(callout.iter().map(|c| c.title.unit));
                        walk(blocks, n);
                    }
                    BlockKind::Code(c) => {
                        n.0.extend((0..c.lines().count() as u32).map(|l| c.first_unit + l));
                        n.2.push(c.id);
                    }
                    BlockKind::Table(t) => {
                        n.0.extend(t.cells.iter().map(|c| c.unit));
                        n.1.push(t.id);
                    }
                    BlockKind::Image { unit, .. } => n.0.push(*unit),
                    BlockKind::Properties(p) => {
                        n.0.extend(p.rows.iter().map(|r| r.value.unit));
                        let c = &p.source;
                        n.0.extend((0..c.lines().count() as u32).map(|l| c.first_unit + l));
                        n.2.push(c.id);
                    }
                    BlockKind::Embed(e) => {
                        n.0.push(e.title.unit);
                        walk(&e.blocks, n);
                    }
                    BlockKind::Rule | BlockKind::Html(_) | BlockKind::FootnoteDefs => {}
                }
            }
        }
        let mut n = Default::default();
        walk(&doc.blocks, &mut n);
        doc.footnotes.iter().for_each(|f| walk(&f.blocks, &mut n));
        n.0.sort_unstable();
        n
    }

    #[test]
    fn a_note_embedded_alone_is_transcluded() {
        let other = "---\ntags: x\n---\n# Other\n\nbody [link](sub/x.md) ![[pic.png]]\n\n\
                     | a | b |\n|---|---|\n| 1 | 2 |\n\n## Part\n\npart text[^n]\n\n\
                     ```rust\nfn x() {}\n```\n\n[^n]: its note\n";
        let embeds = notes(&[("host.md", "other", other)]);
        let host = "# Host\n\nfirst[^h]\n\n![[other]]\n\nafter [[#Host]]\n\n[^h]: the host's\n";
        let doc = transcluded(host, &embeds);
        assert_eq!(
            kinds(&doc.blocks),
            ["heading", "paragraph", "embed", "paragraph", "footnotes"]
        );
        let e = embedded(&doc.blocks[2]);
        assert_eq!(doc.blocks[2].src, LineRange::line(5));
        assert_eq!(doc.plain_text(&e.title), "other");
        assert_eq!(
            kinds(&e.blocks),
            [
                "properties",
                "heading",
                "paragraph",
                "table",
                "heading",
                "paragraph",
                "code"
            ]
        );
        // its links are found from it, its anchors aren't the host's
        let x = doc
            .links
            .iter()
            .position(|l| &*l.dest == "sub/x.md")
            .unwrap() as u32;
        assert_eq!(doc.link_note(x), Some(e.note as usize));
        assert_eq!(
            doc.link_source(x, Path::new("host.md")),
            Path::new("other.md")
        );
        assert_eq!(
            doc.link_source(0, Path::new("host.md")),
            Path::new("host.md")
        );
        assert_eq!(crate::links::find_anchor(&doc, "Part"), None);
        assert_eq!(crate::links::find_anchor(&doc, "Host"), Some(0));
        let part = crate::links::find_anchor_in(&doc, "Part", Some(e.note as usize));
        assert_eq!(doc.anchors[part.unwrap() as usize].text.as_ref(), "Part");
        // numbered on from the host's, each once: units, tables, code blocks
        let (units, tables, codes) = numbered(&doc);
        assert_eq!(units, (0..doc.unit_count).collect::<Vec<_>>());
        assert_eq!((tables, doc.table_count), (vec![0], 1));
        assert_eq!((codes, doc.code_count), (vec![0, 1], 2));
        assert!(
            doc.code_blocks()
                .iter()
                .enumerate()
                .all(|(i, c)| c.id == i as u32)
        );
        // its footnote joins the host's, numbered in order of reference
        let numbers: Vec<_> = doc
            .footnotes
            .iter()
            .map(|f| (&*f.label, f.number))
            .collect();
        assert_eq!(numbers, [("h", 1), ("n", 2)]);
        let ref_run = (doc.runs.iter()).find_map(|r| match r.kind {
            RunKind::FootnoteRef(1) => Some(r.link),
            _ => None,
        });
        assert_eq!(
            doc.links[ref_run.flatten().unwrap() as usize].kind,
            LinkKind::Footnote(1)
        );
        assert!(doc.math.is_empty());
    }

    #[test]
    fn an_embeds_heading_takes_its_section() {
        let other =
            "# Other\n\nintro\n\n## Part\n\npart\n\n### Deeper\n\ndeep\n\n## Next\n\nnext\n";
        let embeds = notes(&[("host.md", "other", other)]);
        let doc = transcluded("![[other#Part]]\n\n![[other#Next]]\n", &embeds);
        let section = |b: &Block| -> Vec<String> {
            embedded(b).blocks.iter().map(|b| shown(&doc, b)).collect()
        };
        assert_eq!(section(&doc.blocks[0]), ["Part", "part", "Deeper", "deep"]);
        assert_eq!(section(&doc.blocks[1]), ["Next", "next"]);
        // a section no heading names, and a block no id names, are links
        let doc = transcluded("![[other#Gone]]\n\n![[other#^id]]\n", &embeds);
        assert_eq!(kinds(&doc.blocks), ["paragraph", "paragraph"]);
    }

    #[test]
    fn an_embeds_block_id_takes_its_block() {
        let other = "# Other\n\nfirst ^para\n\n- one\n  - two ^item\n    - three\n- four\n\n\
                     | a | b |\n|---|---|\n| c | d |\n\n^table\n\n> quoted\n> lines ^quoted\n";
        let embeds = notes(&[("host.md", "other", other)]);
        let host = "![[other#^para]]\n\n![[other#^item]]\n\n![[other#^table]]\n\n\
                    ![[other#^quoted]]\n\n![[other#^PARA]]\n";
        for dialect in [
            Dialect::CommonMark,
            Dialect::Obsidian { line_breaks: false },
        ] {
            let doc = parse_with(host, dialect, Path::new("host.md"), &embeds);
            let obsidian = matches!(dialect, Dialect::Obsidian { .. });
            let blocks = |i: usize| &embedded(&doc.blocks[i]).blocks;
            let para = if obsidian { "first" } else { "first ^para" };
            assert_eq!(kinds(blocks(0)), ["paragraph"]);
            assert_eq!(shown(&doc, &blocks(0)[0]), para);
            // the item and its children, as a list of its own, not indented code
            let items = &list(&blocks(1)[0]).items;
            assert_eq!(items.len(), 1);
            let two = if obsidian { "two" } else { "two ^item" };
            assert_eq!(shown(&doc, &items[0].blocks[0]), two);
            assert_eq!(kinds(&items[0].blocks), ["paragraph", "list"]);
            assert_eq!(kinds(blocks(2)), ["table"]);
            // a quote's paragraph alone
            assert_eq!(kinds(blocks(3)), ["paragraph"]);
            // an id's case
            assert_eq!(shown(&doc, &blocks(4)[0]), para);
        }
    }

    /// The block ids of `doc`: each one's id, its block's first unit and its lines.
    fn block_ids(doc: &Doc) -> Vec<(&str, UnitId, LineRange)> {
        (doc.anchors.iter())
            .filter_map(|a| match a.kind {
                AnchorKind::Block { unit, lines } => Some((&*a.text, unit, lines)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn block_ids_name_their_blocks_and_hide_in_obsidian() {
        let md = "A paragraph. ^para-1\n\n\
                  - item\n- item with **markup** ^item\n  - child\n\n\
                  | a |\n|---|\n| b |\n\n^table\n\n\
                  Two lines\n^own-line\n\n\
                  > quoted ^in-quote\n\n\
                  [[link]] ^after-link\n";
        let obsidian = Dialect::Obsidian { line_breaks: false };
        let doc = parse_as(md, obsidian);
        let paras: Vec<String> = (doc.blocks.iter())
            .filter(|b| matches!(b.kind, BlockKind::Paragraph(_)))
            .map(|b| shown(&doc, b))
            .collect();
        assert_eq!(paras, ["A paragraph.", "Two lines", "link"]);
        assert_eq!(
            kinds(&doc.blocks),
            [
                "paragraph",
                "list",
                "table",
                "paragraph",
                "quote",
                "paragraph"
            ]
        );
        let items = &list(&doc.blocks[1]).items;
        assert_eq!(shown(&doc, &items[1].blocks[0]), "item with markup");
        let BlockKind::Quote { blocks, .. } = &doc.blocks[4].kind else {
            panic!()
        };
        assert_eq!(shown(&doc, &blocks[0]), "quoted");
        // the paragraph of the id on its own line ends a line sooner
        assert_eq!(doc.blocks[3].src, LineRange::new(13, 13));
        let ids = block_ids(&doc);
        let names: Vec<_> = ids.iter().map(|&(name, _, _)| name).collect();
        assert_eq!(
            names,
            [
                "para-1",
                "item",
                "table",
                "own-line",
                "in-quote",
                "after-link"
            ]
        );
        // the item's lines, its child's too (and the blank line after, the list's last); the
        // table's, which the id alone after it names
        assert_eq!((ids[1].2.start, ids[1].2.end.min(5)), (4, 5));
        assert_eq!(ids[2].2, LineRange::new(7, 9));
        let find = |frag: &str| crate::links::find_anchor(&doc, frag);
        for (name, _, _) in &ids {
            let found = find(&format!("^{name}")).map(|a| &*doc.anchors[a as usize].text);
            assert_eq!(found, Some(*name));
        }
        assert_eq!(find("^ITEM"), find("^item"));
        assert_eq!(find("^gone"), None);
        // a heading's slug names no block
        assert_eq!(find("para-1"), None);
        // an id alone with no block before it is text
        let doc = parse_as("^alone\n\ntext\n", obsidian);
        assert_eq!(block_ids(&doc), []);
        assert_eq!(shown(&doc, &doc.blocks[0]), "^alone");

        // in CommonMark, shown, and still found
        let doc = parse(md);
        let first = match &doc.blocks[0].kind {
            BlockKind::Paragraph(_) => shown(&doc, &doc.blocks[0]),
            _ => panic!(),
        };
        assert_eq!(first, "A paragraph. ^para-1");
        assert_eq!(block_ids(&doc).len(), 6);
        assert_eq!(block_ids(&doc)[2].2, LineRange::new(7, 9));
    }

    #[test]
    fn what_isnt_a_block_id() {
        let md = "text^glued\n\n**bold**^glued\n\n`code ^id`\n\nescaped \\^id\n\n\
                  [link ^id](x)\n\nnot ^an_id\n\nnot ^\n\n# Heading ^no\n";
        let doc = parse_as(md, Dialect::Obsidian { line_breaks: false });
        assert_eq!(block_ids(&doc), []);
        let shown: Vec<String> = (doc.blocks.iter()).map(|b| shown(&doc, b)).collect();
        assert_eq!(
            shown,
            [
                "text^glued",
                "bold^glued",
                "code ^id",
                "escaped ^id",
                "link ^id",
                "not ^an_id",
                "not ^",
                "Heading ^no"
            ]
        );
    }

    #[test]
    fn only_an_embed_alone_in_its_paragraph_and_read_is_transcluded() {
        let embeds = notes(&[("host.md", "other", "body\n")]);
        let doc = transcluded(
            "see ![[other]] here\n\n![[unread]]\n\n- ![[other]]\n",
            &embeds,
        );
        assert_eq!(kinds(&doc.blocks), ["paragraph", "paragraph", "list"]);
        assert_eq!(kinds(&list(&doc.blocks[2]).items[0].blocks), ["embed"]);
        // without the notes, as `parse` reads it, the embed is its link
        assert_eq!(kinds(&parse("![[other]]\n").blocks), ["paragraph"]);
    }

    #[test]
    fn a_note_is_never_transcluded_inside_itself() {
        let embeds = notes(&[
            ("host.md", "a", "a1\n\n![[b]]\n"),
            ("a.md", "b", "b1\n\n![[a]]\n\n![[host]]\n"),
            ("b.md", "a", "a1\n\n![[b]]\n"),
            ("b.md", "host", "host\n"),
            ("host.md", "host", "host\n"),
        ]);
        let doc = transcluded("![[a]]\n\n![[host]]\n", &embeds);
        let a = embedded(&doc.blocks[0]);
        assert_eq!(kinds(&a.blocks), ["paragraph", "embed"]);
        let b = embedded(&a.blocks[1]);
        // `a` and the host are being parsed: their embeds stay links
        assert_eq!(kinds(&b.blocks), ["paragraph", "paragraph", "paragraph"]);
        assert_eq!(kinds(&doc.blocks[1..]), ["paragraph"]);
        // the notes nest: b's in a's
        let paths: Vec<_> = doc
            .embedded
            .iter()
            .map(|e| e.path.to_str().unwrap())
            .collect();
        assert_eq!(paths, ["b.md", "a.md"]);
        assert!(doc.embedded[1].links.start <= doc.embedded[0].links.start);
        assert!(doc.embedded[0].links.end <= doc.embedded[1].links.end);
    }

    #[test]
    fn embeds_nest_only_so_deep() {
        let chain: Vec<(String, String, String)> = (0..8)
            .map(|i| {
                let from = if i == 0 {
                    "host.md".into()
                } else {
                    format!("n{}.md", i - 1)
                };
                (from, format!("n{i}"), format!("![[n{}]]\n", i + 1))
            })
            .collect();
        let refs: Vec<_> = chain.iter().map(|(f, n, t)| (&**f, &**n, &**t)).collect();
        let doc = transcluded("![[n0]]\n", &notes(&refs));
        let mut depth = 0;
        let mut blocks = &doc.blocks;
        while let Some(BlockKind::Embed(e)) = blocks.first().map(|b| &b.kind) {
            depth += 1;
            blocks = &e.blocks;
        }
        assert_eq!(depth, embed::MAX_DEPTH);
    }

    /// A false `$` closer parses the note again; the embeds the first pass grafted before it
    /// count once toward the bytes a parse transcludes, not once a pass (PAR-012).
    #[test]
    fn a_false_closer_counts_embeds_once() {
        let big = "words and words\n\n".repeat((11 << 20) / 17);
        let embeds = notes(&[("host.md", "big", &big)]);
        // 2 × 11 MiB fit under the cap, 3 × 11 MiB (two again past the closer) wouldn't
        let doc = transcluded("![[big]]\n\n![[big]]\n\nit costs $1 or ($2 off\n", &embeds);
        assert_eq!(kinds(&doc.blocks)[..2], ["embed", "embed"]);
        assert!(doc.math.is_empty());
    }

    #[test]
    fn image_embeds_stay_images() {
        let doc =
            parse("![[pic.png]]\n\n![[pic.png|100]]\n\n![x](https://e.com/a.md)\n\n![y](badge)\n");
        assert_eq!(kinds(&doc.blocks), ["image"; 4]);
        assert!(doc.links.iter().all(|l| l.kind == LinkKind::Image));
    }

    #[test]
    fn obsidians_tags_are_styled() {
        let obsidian = Dialect::Obsidian { line_breaks: false };
        let text = "#tag at the start, a #nested/tag-2 and #my_tag_too, #emoji🦀 and #日本語.\n\
                    No #12, C#, a#b, ##, `#code`, [#link](x), https://e.com/#frag, x(#paren), #️⃣.\n\
                    #读书，and #a7）を、 #ｆｗ！\n\
                    #2024-01 and *#emph* and **x**#after and #tag#two. \\#include, \\#!/bin/sh\n\n\
                    ## Heading #h\n\n\
                    | #cell |\n|---|\n| #row |\n\n\
                    > [!note] Title #t\n";
        // each tag whole, its runs joined: pulldown-cmark splits `#my_tag_too` at its `_`s
        let tags = |doc: &Doc| {
            let mut tags: Vec<String> = Vec::new();
            let mut end = None;
            for r in &doc.runs {
                if !r.style.contains(InlineStyle::TAG) {
                    end = None;
                    continue;
                }
                match tags.last_mut() {
                    Some(tag) if end == Some((r.text.start, r.src_line)) => {
                        tag.push_str(doc.run_text(r));
                    }
                    _ => tags.push(doc.run_text(r).into()),
                }
                end = Some((r.text.end, r.src_line));
            }
            tags
        };
        let doc = parse_as(text, obsidian);
        assert_eq!(
            tags(&doc),
            [
                "#tag",
                "#nested/tag-2",
                "#my_tag_too",
                "#emoji🦀",
                "#日本語",
                "#读书",
                "#a7",
                "#ｆｗ",
                "#2024-01",
                "#emph",
                "#tag",
                "#h",
                "#cell",
                "#row",
                "#t",
            ]
        );
        // the text is unchanged, and a tag in emphasis keeps it
        assert!(shown(&doc, &doc.blocks[0]).starts_with("#tag at the start, a #nested/tag-2"));
        let runs = doc
            .runs
            .iter()
            .filter(|r| r.style.contains(InlineStyle::TAG));
        assert!(runs.clone().any(|r| r.style.contains(InlineStyle::EMPH)));
        // CommonMark has no tags
        assert!(tags(&parse(text)).is_empty());
    }

    #[test]
    fn image_sizes_come_off_the_alt_text() {
        let size = |width, height| ImageSize { width, height };
        let doc = parse(
            "![[a.png|100]]\n\n![[b.png|Alt text|100x145]]\n\n![Alt|200](c.png)\n\n\
             ![[d.png|Caption]]\n\n![100](e.png)\n\n![a|b](f.png)\n\n![x|0](g.png)\n\n\
             in ![a|50](h.png) text\n\n![[note|100]]\n",
        );
        let images: Vec<_> = (doc.blocks.iter())
            .filter_map(|b| match &b.kind {
                BlockKind::Image { link, alt, .. } => {
                    Some((&**alt, doc.links[*link as usize].size))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            images,
            [
                // as `![[a.png]]` reads
                ("a.png", size(100, 0)),
                ("Alt text", size(100, 145)),
                ("Alt", size(200, 0)),
                ("Caption", ImageSize::NONE),
                // a markdown image's size needs its `|`
                ("100", ImageSize::NONE),
                ("a|b", ImageSize::NONE),
                ("x|0", ImageSize::NONE),
            ]
        );
        let runs = paragraph_runs(&doc).concat();
        assert!(runs.contains(&("a", RunKind::InlineImage(7))));
        assert_eq!(doc.links[7].size, size(50, 0));
        // a note's embed has no size: its alias is its text
        assert!(runs.contains(&("100", RunKind::Embed(8))));
    }

    #[test]
    fn image_sizes_cap_and_never_enlarge() {
        let size = |width, height| ImageSize { width, height };
        assert_eq!(ImageSize::parse("100"), Some(size(100, 0)));
        assert_eq!(ImageSize::parse(" 100 x 145 "), Some(size(100, 145)));
        assert_eq!(ImageSize::parse("99999999999"), Some(size(65535, 0)));
        for s in ["", "x", "0", "0x10", "10x", "-1", "1.5", "100px"] {
            assert_eq!(ImageSize::parse(s), None, "{s:?}");
        }
        assert_eq!(size(100, 0).cap((400, 300)), (100, 75));
        assert_eq!(size(100, 50).cap((400, 300)), (67, 50));
        assert_eq!(size(0, 30).cap((400, 300)), (40, 30));
        assert_eq!(size(1000, 0).cap((400, 300)), (400, 300));
        assert_eq!(ImageSize::NONE.cap((400, 300)), (400, 300));
        assert_eq!(size(1, 0).cap((400, 3)), (1, 1));
    }

    #[test]
    fn front_matter_is_read_into_properties() {
        let doc = parse("---\ntitle: x\n---\n\n# Body\n");
        assert_eq!(kinds(&doc.blocks), ["properties", "heading"]);
        assert_eq!(doc.blocks[0].src, LineRange::new(1, 3));
        assert_eq!(doc.blocks[1].src, LineRange::line(5));
        // the note has an H1, so the title doesn't head it
        assert_eq!(doc.properties().unwrap().title.as_deref(), Some("x"));
        assert_eq!(doc.title(), None);
        assert_eq!(parse("---\ntitle: x\n---\n## Body\n").title(), Some("x"));
    }

    fn rows(doc: &Doc) -> Vec<(String, String, LineRange)> {
        let p = doc.properties().unwrap();
        (p.rows.iter())
            .map(|r| (r.key.to_string(), doc.plain_text(&r.value), r.src))
            .collect()
    }

    #[test]
    fn properties_are_rows_of_a_key_and_its_value() {
        let text = "---\ntitle: A note\ntags: [one, '#two']\naliases:\n  - Other\n  - Else\n\
                    up: \"[[Parent|the parent]]\"\nsite: see https://x.org/a.\ncssclasses: wide\n\
                    nested:\n  a: 1\n---\nbody\n";
        let doc = parse(text);
        let row = |k: &str, v: &str, a: u32, b: u32| (k.into(), v.into(), LineRange::new(a, b));
        assert_eq!(
            rows(&doc),
            [
                row("title", "A note", 2, 2),
                row("tags", "#one #two", 3, 3),
                row("aliases", "Other, Else", 4, 6),
                row("up", "the parent", 7, 7),
                row("site", "see https://x.org/a.", 8, 8),
                // `cssclasses` is Obsidian's own, not shown
                row("nested", "a: 1 …", 10, 11),
            ]
        );
        let p = doc.properties().unwrap();
        // tags are styled, links are links
        let runs = doc.runs(&p.rows[1].value);
        assert!(
            runs.iter()
                .all(|r| doc.run_text(r) == " " || r.style == InlineStyle::TAG)
        );
        let link = |row: usize| doc.runs(&p.rows[row].value).iter().find_map(|r| r.link);
        let wiki = &doc.links[link(3).unwrap() as usize];
        assert_eq!(
            (&*wiki.dest, wiki.kind),
            ("Parent", LinkKind::Wiki { aliased: true })
        );
        let url = &doc.links[link(4).unwrap() as usize];
        assert_eq!(
            (&*url.dest, url.kind),
            ("https://x.org/a", LinkKind::Autolink)
        );
        // the source is a code block of its own, in YAML
        assert_eq!(p.source.lang.as_deref(), Some("yaml"));
        assert_eq!(p.source.first_line, 2);
        assert_eq!(doc.code_blocks().len(), 1);
        assert!(p.read);
    }

    #[test]
    fn unreadable_and_toml_front_matter() {
        let doc = parse("---\n- a list\n---\nbody\n");
        let p = doc.properties().unwrap();
        assert!(!p.read && p.rows.is_empty());
        assert_eq!(&*p.source.text, "- a list\n");
        // TOML, Hugo's, read too; before, the `+++` was text
        let doc = parse("+++\ntitle = \"Hi\"\ndraft = false\n+++\nbody\n");
        assert_eq!(kinds(&doc.blocks), ["properties", "paragraph"]);
        let p = doc.properties().unwrap();
        assert_eq!(p.source.lang.as_deref(), Some("toml"));
        assert_eq!(
            rows(&doc)[1],
            ("draft".into(), "false".into(), LineRange::line(3))
        );
        assert_eq!(doc.title(), Some("Hi"));
    }

    /// pulldown-cmark's option took any `---` starting a block, then text, for front matter up
    /// to the next `---`, and the text between was hidden.
    #[test]
    fn a_rule_then_text_in_a_note_is_no_front_matter() {
        let text = "intro\n\n---\nshown\n\nmore\n---\n## Section\n\n---\nlast\n...\n";
        let doc = parse(text);
        assert_eq!(
            kinds(&doc.blocks),
            [
                "paragraph",
                "rule",
                "paragraph",
                "heading",
                "heading",
                "rule",
                "paragraph"
            ]
        );
        let texts: Vec<_> = (doc.blocks.iter())
            .filter(|b| !matches!(b.kind, BlockKind::Rule))
            .map(|b| shown(&doc, b))
            .collect();
        assert_eq!(texts, ["intro", "shown", "more", "Section", "last ..."]);
        assert_eq!(doc.blocks[3].src, LineRange::new(6, 7));
    }

    #[test]
    fn front_matter_after_a_blank_line_is_obsidians() {
        let obsidian = Dialect::Obsidian { line_breaks: false };
        let text = "---\n\ntitle: x\n---\n\n# Body %%c%%\n";
        let doc = parse_as(text, obsidian);
        assert_eq!(kinds(&doc.blocks), ["properties", "heading"]);
        assert_eq!(doc.blocks[1].src, LineRange::line(6));
        assert_eq!(shown(&doc, &doc.blocks[1]), "Body");
        assert_eq!(
            rows(&doc),
            [("title".into(), "x".into(), LineRange::line(3))]
        );
        // CommonMark: a rule, then a setext heading
        assert_eq!(kinds(&parse(text).blocks), ["rule", "heading", "heading"]);
        // a `%%` in it opens no comment
        let doc = parse_as("---\n\nx: '%%'\n---\nseen %%not%%\n", obsidian);
        assert_eq!(rows(&doc)[0].1, "%%");
        assert_eq!(shown(&doc, &doc.blocks[1]).trim_end(), "seen");
        // an Excalidraw drawing's, in any dialect: its notice in place of it (OBS-015)
        let text = "---\n\nexcalidraw-plugin: parsed\n\n---\n==⚠ Switch to EXCALIDRAW VIEW==\n";
        let doc = parse(text);
        assert_eq!(kinds(&doc.blocks), ["quote"]);
        assert_eq!(doc.blocks[0].src, LineRange::new(1, 6));
    }

    /// An Excalidraw drawing (OBS-015): a notice, then its text as Obsidian's dialect reads it,
    /// in any, the plugin's banner and the drawing's data left out.
    #[test]
    fn a_drawing_is_its_notice_and_its_text() {
        let text = "---\n\nexcalidraw-plugin: parsed\ntags: [excalidraw]\n\n---\n\
                    ==⚠  Switch to EXCALIDRAW VIEW in the MORE OPTIONS menu. ⚠==\n\n\n\
                    # Excalidraw Data\n## Text Elements\ntwo\nlines ^abcd1234\n\n\
                    %%\n## Drawing\n```compressed-json\nN4Ig\n```\n%%";
        for doc in [
            parse(text),
            parse_as(text, Dialect::Obsidian { line_breaks: false }),
        ] {
            assert_eq!(
                kinds(&doc.blocks),
                ["quote", "heading", "heading", "paragraph"]
            );
            let BlockKind::Quote {
                callout: Some(c), ..
            } = &doc.blocks[0].kind
            else {
                panic!()
            };
            assert_eq!(c.kind, CalloutKind::Info);
            assert_eq!(doc.blocks[0].src, LineRange::new(1, 7));
            // the id hidden, and naming its text
            assert_eq!(block_ids(&doc)[0].0, "abcd1234");
            assert_eq!(doc.blocks[3].src, LineRange::new(12, 13));
            assert!(!shown(&doc, &doc.blocks[3]).contains('^'));
        }
        // in CommonMark, its lines kept, as Obsidian's default keeps them
        let doc = parse(text);
        let BlockKind::Paragraph(p) = &doc.blocks[3].kind else {
            panic!()
        };
        let kinds: Vec<_> = doc.runs(p).iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RunKind::Text, RunKind::HardBreak, RunKind::Text]);
    }

    /// Quotes and lists past `MAX_NESTING` fold into the last, keeping what they hold.
    #[test]
    fn nesting_stops_at_its_limit() {
        fn depth(blocks: &[Block]) -> usize {
            (blocks.iter())
                .map(|b| match &b.kind {
                    BlockKind::Quote { blocks, .. } => 1 + depth(blocks),
                    BlockKind::List(l) => {
                        1 + l.items.iter().map(|i| depth(&i.blocks)).max().unwrap_or(0)
                    }
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }
        let quotes = parse(&format!("{}a\n", ">".repeat(10_000)));
        assert_eq!(depth(&quotes.blocks), MAX_NESTING);
        let mixed = parse(&format!("{}[ ] a\n", "> - ".repeat(10_000)));
        assert_eq!(depth(&mixed.blocks), MAX_NESTING);
        for doc in [quotes, mixed] {
            let mut tasks = 0;
            let mut text = String::new();
            doc.each_block(&mut |b| match &b.kind {
                BlockKind::List(l) => tasks += l.items.iter().filter(|i| i.task.is_some()).count(),
                BlockKind::Paragraph(inl) => text.push_str(&doc.plain_text(inl)),
                _ => {}
            });
            // the folded item's marker is its text's, not an outer item's
            assert_eq!(tasks, 0);
            assert!(text.ends_with('a'), "{text:?}");
        }
    }

    #[test]
    fn hostile_input_does_not_panic() {
        for text in [
            "",
            "\n\n\n",
            "- [ ]",
            "[^x]",
            "> - > - > ```\n",
            "|a|\n|-|\n|![x](y)|",
            "<br>",
            "***\n---\n___",
            "[[]]",
            "\u{0}\u{feff}#",
        ] {
            parse(text);
        }
    }
}
