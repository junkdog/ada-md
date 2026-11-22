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
use super::lines::{code_line_count, line_starts_and_marks};
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
/// not by pulldown-cmark's option, which takes a `---` anywhere for it.
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
/// in math, often a prime (`` $f`(x)$ ``). pulldown-cmark has no hook for its closers, so a text
/// where it closes math on such a `$` is parsed again with that `$` masked (a byte the text
/// lacks, of the same length, so every offset holds), until no such `$` is left or after
/// [`MAX_MATH_PASSES`]. Notes without one parse once, as before.
///
/// In Obsidian's dialect, its `%%comments%%` go first (OBS-004), their line ends kept: what's
/// parsed is the note without them, its lines numbered as the note's. pulldown-cmark reads the
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
    let uncommented;
    let text = match obsidian {
        true => {
            uncommented = super::comments::strip(text, max)?;
            &*uncommented
        }
        false => text,
    };
    // comments start past the front matter, which keeps its place
    let front = match &drawing {
        Some(d) => d.start,
        None => read.filter(|f| f.taken(obsidian)).map_or(0, |f| f.end),
    };
    let at = |range: Range<usize>| range.start + front..range.end + front;
    let mut source = Cow::Borrowed(text);
    let mut mask = None;
    let mut maskable = true;
    // what the notes transcluded before this parse took: a pass after a false closer grafts its
    // embeds again, and counts them from here, not on top of the last pass's (PAR-012)
    let spent = embedding.map(|e| e.budget.spent.get());
    for pass in 0.. {
        let check = maskable && pass < MAX_MATH_PASSES;
        let mut b = Builder::new(text);
        b.embedding = embedding;
        // a drawing's text elements keep their lines, as Obsidian's default does
        b.line_breaks = match dialect {
            Dialect::Obsidian { .. } => dialect.line_breaks(),
            _ => drawing.is_some(),
        };
        b.obsidian = obsidian;
        if drawing.is_some() {
            b.drawing(front);
        } else if front > 0 {
            b.properties();
        }
        let mut events = Parser::new_ext(&source[front..], options())
            .into_offset_iter()
            .enumerate();
        let mut wrong = Vec::new();
        for (n, (event, range)) in events.by_ref() {
            if n == max {
                return None;
            }
            let range = at(range);
            if check && matches!(event, Event::InlineMath(_)) && !math_closes(text, &range) {
                wrong.push(range.end - 1);
                break;
            }
            b.event(unmask(event, mask), range);
        }
        if wrong.is_empty() {
            return Some(b.finish());
        }
        // the rest of the pass's false closers, so that the next pass masks them all
        for (n, (event, range)) in events {
            if n == max {
                return None;
            }
            let range = at(range);
            if matches!(event, Event::InlineMath(_)) && !math_closes(text, &range) {
                wrong.push(range.end - 1);
            }
        }
        if let (Some(e), Some(spent)) = (embedding, spent) {
            e.budget.spent.set(spent);
        }
        let Some(m) = mask.or_else(|| unused_byte(text)) else {
            // a text holding every candidate byte (only a fuzzer's): math as pulldown-cmark reads it
            maskable = false;
            continue;
        };
        mask = Some(m);
        let mut bytes = source.into_owned().into_bytes();
        for at in wrong {
            bytes[at] = m;
        }
        // `$` and the mask are both ASCII
        source = Cow::Owned(String::from_utf8(bytes).ok()?);
    }
    None
}

/// The notice above an Excalidraw drawing's text (OBS-015).
const DRAWING: &str = "An Excalidraw drawing, shown as its text";

/// The most times [`markdown`] parses a text again for math closed on a `$` it shouldn't be:
/// each pass masks every such `$` it meets, and a pass only meets new ones where masking let an
/// opener reach further. The last pass takes what it gets.
const MAX_MATH_PASSES: usize = 4;

/// Whether the inline math at `range` (its `$`s included) closes where pulldown-cmark closed it:
/// its closing `$` not followed by a digit (pandoc's rule), nor right after a backtick unless
/// the math opened on one (`` $`…`$ ``).
fn math_closes(text: &str, range: &Range<usize>) -> bool {
    let b = text.as_bytes();
    if range.end < range.start + 3 || b[range.start] != b'$' || b[range.end - 1] != b'$' {
        return true;
    }
    let tick_closes = b[range.end - 2] != b'`' || b[range.start + 1] == b'`';
    !b.get(range.end).is_some_and(u8::is_ascii_digit) && tick_closes
}

/// A byte `text` doesn't hold, to mask a `$` with. First ASCII punctuation that means nothing to
/// CommonMark, GFM or the extensions on, as `$` (outside math) doesn't: punctuation, as `$` is, so
/// emphasis next to it flanks the same, and a link's destination may hold it. Then a control
/// byte, which a link's destination may not (a masked `$` in one breaks the link, as rare as a
/// text holding all four): not a tab, line feed, form feed or carriage return, nor NUL, which
/// pulldown-cmark replaces.
fn unused_byte(text: &str) -> Option<u8> {
    let mut seen = [false; 128];
    for &c in text.as_bytes() {
        if c < 128 {
            seen[usize::from(c)] = true;
        }
    }
    (b"%?,@".iter().copied())
        .chain((1..32u8).filter(|c| !matches!(c, b'\t'..=b'\r')))
        .find(|&c| !seen[usize::from(c)])
}

/// `event` with each masked `$` written back. A masked `$` is where math stood in an earlier
/// pass, inline, so it can land in text, code, math, HTML, a footnote's label or a link's parts.
fn unmask(event: Event<'_>, mask: Option<u8>) -> Event<'_> {
    let Some(m) = mask else {
        return event;
    };
    fn back(s: CowStr<'_>, m: u8) -> CowStr<'_> {
        if s.as_bytes().contains(&m) {
            s.replace(char::from(m), "$").into()
        } else {
            s
        }
    }
    let un = |s| back(s, m);
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
        other => other,
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
    /// Obsidian's dialect: GitHub's `important` and `caution` are its tip and warning (OBS-002).
    obsidian: bool,
    /// What opens the quote or item just opened, being read: a callout's marker (OBS-002) or a
    /// task status past GFM's (OBS-012). One field, so an event pays one test for both.
    opening: Option<Opening>,
    /// Where each run of two or more `=` in the text has its second: a closing frame's runs may
    /// hold a highlight only when one is on their lines (OBS-005).
    marks: Vec<u32>,
    /// The first of [`Builder::marks`] at or past the last frame [`Builder::marked`] looked at.
    mark: usize,
    /// Where each escaped `#` (`\#word`) is in [`Builder::text`], in Obsidian's dialect: no tag
    /// opens there (OBS-006).
    escaped: Vec<u32>,
    /// Typeset math's TeX (OBS-003), [`Doc::math`].
    math: Vec<TextRange>,
    /// Where embeds are transcluded from (OBS-007); `None` leaves them links.
    embedding: Option<Embedding<'s>>,
    /// The notes transcluded, [`Doc::embedded`].
    embedded: Vec<Embedded>,
}

impl<'s> Builder<'s> {
    fn new(text: &'s str) -> Self {
        let (line_starts, marks) = line_starts_and_marks(text);
        Self {
            source: text,
            line_starts,
            // the source's length, which inline text rarely passes: grown by doubling instead,
            // a long run's text was copied at each step (long lines' parse +5–10%, experiment
            // 71); the room left over is never touched, and `finish` gives it back
            text: String::with_capacity(text.len()),
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
            obsidian: false,
            opening: None,
            marks,
            mark: 0,
            escaped: Vec::new(),
            math: Vec::new(),
            embedding: None,
            embedded: Vec::new(),
        }
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
        let event = match &self.opening {
            Some(Opening::Callout(_)) => match self.read_marker(event, &range, src) {
                Some(event) => event,
                None => return,
            },
            Some(Opening::Status(_)) => match self.read_status(event, &range) {
                Some(event) => event,
                None => return,
            },
            None => event,
        };
        match event {
            Event::Start(Tag::BlockQuote(None)) if self.nesting < MAX_NESTING => {
                self.start(Tag::BlockQuote(None), src);
                self.opening =
                    marker(self.source, range.start, self.obsidian).map(Opening::Callout);
            }
            Event::Start(tag) => {
                // a test of the tag's kind, not an arm of its own: that one dropped each item's
                // `Tag` apart, and the lists input parsed in 0.4% more instructions
                let item = matches!(tag, Tag::Item);
                self.start(tag, src);
                if item {
                    self.opening = status(self.source, range).map(Opening::Status);
                }
            }
            Event::End(end) => self.end(end),
            Event::Text(t) if self.obsidian && t.starts_with('#') => {
                let at = self.text_end();
                self.text(&t, line);
                // `\#word` is no tag (OBS-006): pulldown-cmark makes the escaped `#` text of its own
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
            Event::Text(t) => self.text(&t, line),
            Event::Code(t) => self.push_text(&t, self.style | InlineStyle::CODE, line),
            // typeset as Unicode, else the TeX as written, as code (OBS-003)
            Event::InlineMath(t) => match tex::inline(&t) {
                Some(math) => self.push_math(&math, &t, line),
                None => self.push_text(&t, self.style | InlineStyle::CODE, line),
            },
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

    fn start(&mut self, tag: Tag<'_>, src: LineRange) {
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
                    LinkType::WikiLink { has_pothole } => LinkKind::Wiki {
                        aliased: has_pothole,
                    },
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

    fn push_text_as(&mut self, text: &str, style: InlineStyle, line: u32, kind: RunKind) {
        if self.image.is_some() {
            self.text.push_str(text);
            return;
        }
        if mem::take(&mut self.after_math)
            && let Some(Frame::Inline { src, .. }) = self.stack.last_mut()
        {
            // the rest of a paragraph split by display math starts where its text does
            *src = LineRange::new(line, src.end.max(line));
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

    fn push_break(&mut self, kind: RunKind, line: u32) {
        if self.image.is_some() {
            self.text.push(' ');
            return;
        }
        if self.after_math {
            return;
        }
        if let Some(Frame::Inline {
            kind: InlineKind::Title,
            ..
        }) = self.stack.last()
            && self.end_title(Some(line))
        {
            // the line's end ended the title, and the rest of the paragraph is the body's
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
        if !self.marks.is_empty() && self.marked(start as usize) {
            self.highlight(start as usize);
        }
        if self.obsidian {
            self.tags(start as usize);
        }
        let end = self.runs.len() as u32;
        debug_assert_eq!(
            self.inlines_open as usize,
            (self.stack.iter())
                .filter(|f| matches!(f, Frame::Inline { .. }))
                .count()
        );
        if self.inlines_open == 0 {
            return Slots { start, end };
        }
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

    /// Whether a run of `=`s is on the lines of the inline frame closing, whose runs start at
    /// `start`: only then may it hold a highlight. Found from [`Builder::mark`] on, as frames
    /// close in source order, nearly (a binary search a frame cost a note with one `==` 1–2% of
    /// its parse).
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
        self.mark = i;
        marks.get(i).is_some_and(|&q| q < to)
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
        self.links.push(LinkRef {
            dest: dest.into(),
            title: title.into(),
            kind,
            src_line: line,
            size: ImageSize::NONE,
        });
        self.links.len() as LinkId - 1
    }

    fn footnote_id(&mut self, label: &str) -> FootnoteId {
        if let Some(&id) = self.footnote_ids.get(label) {
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
        self.footnote_ids.insert(label.into(), id);
        id
    }

    fn pop_block(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        if let Some(Frame::Inline {
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
                        self.embed(start, src)
                            .unwrap_or_else(|| self.paragraph(start, src))
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
        let mut chars = name.chars();
        let text = self.text_end();
        if let Some(first) = chars.next() {
            self.text.extend(first.to_uppercase());
            self.text.extend(chars.flat_map(char::to_lowercase));
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
    #[inline]
    fn block_id(&self, start: u32) -> Option<BlockId> {
        let last = self.runs[start as usize..].last()?;
        // bytes, back from the end: most paragraphs end in a word or a stop, which no `^` heads
        // (a paragraph's check was ~170 instructions with `str`'s trims, the lists input +5%)
        let bytes = last.text.of(&self.text).as_bytes();
        let end = bytes.iter().rposition(|&b| b != b' ' && b != b'\t')? + 1;
        let caret =
            (bytes[..end].iter()).rposition(|&b| !(b.is_ascii_alphanumeric() || b == b'-'))?;
        if bytes[caret] != b'^' || caret + 1 == end {
            return None;
        }
        self.block_id_at(start, caret, end)
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
    /// that holds nothing but one image becomes an image block, and its runs go.
    fn paragraph(&mut self, start: u32, src: LineRange) -> Block {
        let unit = self.next_unit();
        let mut images = self.runs[start as usize..]
            .iter()
            .filter(|r| !is_blank(r, &self.text));
        if let (Some(run), None) = (images.next(), images.next())
            && let RunKind::InlineImage(link) = run.kind
        {
            let alt = run.text.of(&self.text).into();
            self.runs.truncate(start as usize);
            return Block {
                kind: BlockKind::Image { link, alt, unit },
                src,
            };
        }
        Block {
            kind: BlockKind::Paragraph(Inlines {
                runs: self.close_runs(start),
                unit,
            }),
            src,
        }
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
/// the title. Only offsets: the parser's events confirm it ([`Builder::read_marker`]).
fn marker(source: &str, at: usize, obsidian: bool) -> Option<Marker> {
    let b = source.as_bytes();
    let blank = |i: &mut usize| {
        while b.get(*i).is_some_and(|&c| c == b' ' || c == b'\t') {
            *i += 1;
        }
    };
    let mut i = at;
    blank(&mut i);
    if b.get(i) != Some(&b'>') {
        return None;
    }
    i += 1;
    blank(&mut i);
    let open = i;
    if b.get(i..i + 2) != Some(b"[!") {
        return None;
    }
    i += 2;
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
/// only in an item with no other `*`, which could pair with it into emphasis.
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
    let boxed = i > marker
        && b.get(i) == Some(&b'[')
        && b.get(i + 2) == Some(&b']')
        && matches!(b.get(i + 3), Some(b' ' | b'\t'));
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
