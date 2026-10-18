//! Session recording and replay (ADR-19, architecture §10 "Recording format").
//!
//! A `.rec` file is a `ada-rec 1` header, then one line per message:
//! `+<ms since start> <kebab-name> <field>*`. A field is `key=value` (an integer, `WxH`, a bare
//! identifier or a `"quoted string"`) or `key:<len>`, in which case `len` raw bytes follow the
//! line, in field order, then a newline. File text is stored verbatim, unescaped.
//!
//! One macro table generates both the writer and the reader's message mapping, so they can't
//! drift; the line grammar is an anpa parser.

use std::collections::VecDeque;
use std::fmt::{Display, Write as _};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anpa::combinators::{
    attempt, fold, left, map, map_if, middle, no_separator, not_empty, or, or_diff, peek, right,
};
use anpa::core::{ParserExtNoState, StrParser};
use anpa::number::integer;
use anpa::parsers::{empty, item, item_if, item_while, skip};
use anpa::{or, tuplify};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::{self, Computed, Dir, Effects, Msg, ScrollBy, Size, State};
use crate::grid::Grid;
use crate::images::{Probe, Proto};
use crate::links::Resolved;
use crate::ui;

pub const HEADER: &str = "ada-rec 1";

macro_rules! messages {
    ($($name:literal => $variant:ident { $($field:ident),* },)*) => {
        fn encode(msg: &Msg, line: &mut String, blobs: &mut Vec<u8>) {
            match msg {
                $(Msg::$variant { $($field),* } => {
                    line.push_str($name);
                    $(Field::write($field, stringify!($field), line, blobs);)*
                })*
            }
        }

        fn decode(name: &str, mut _fields: Fields) -> Result<Msg, String> {
            match name {
                $($name => Ok(Msg::$variant { $($field: _fields.take(stringify!($field))?,)* }),)*
                _ => Err(format!("unknown message `{name}`")),
            }
        }

        #[cfg(test)]
        const NAMES: &[&str] = &[$($name),*];
    };
}

messages! {
    "init" => Init { size },
    "graphics" => Graphics { proto, cell },
    "resize" => Resize { size },
    "open" => Open { path },
    "scroll" => Scroll { by },
    "jump-heading" => JumpHeading { dir },
    "toggle-table-mode" => ToggleTableMode {},
    "sort-open" => SortOpen {},
    "sort-table" => SortTable { dir },
    "toggle-code-wrap" => ToggleCodeWrap {},
    "toggle-line-numbers" => ToggleLineNumbers {},
    "scroll-code-x" => ScrollCodeX { by },
    "focus-link" => FocusLink { dir },
    "follow" => Follow {},
    "back" => Back {},
    "forward" => Forward {},
    "open-outline" => OpenOutline {},
    "open-links" => OpenLinks {},
    "outline-filter" => OutlineFilter { text },
    "outline-move" => OutlineMove { by },
    "outline-pick" => OutlinePick {},
    "outline-close" => OutlineClose {},
    "search-open" => SearchOpen {},
    "search-input" => SearchInput { pattern },
    "search-submit" => SearchSubmit {},
    "search-cancel" => SearchCancel {},
    "search-next" => SearchNext { dir },
    "jump-change" => JumpChange { dir },
    "reload" => Reload {},
    "open-themes" => OpenThemes {},
    "themes-tone" => ThemesTone { dir },
    "help" => Help {},
    "help-scroll" => HelpScroll { by },
    "help-close" => HelpClose {},
    "edit" => Edit {},
    "yank" => Yank {},
    "save-config" => SaveConfig {},
    "save-confirm" => SaveConfirm {},
    "save-cancel" => SaveCancel {},
    "quit" => Quit {},
    "doc-loaded" => DocLoaded { path, mtime, text, doc },
    "doc-failed" => DocFailed { path, error },
    "links-resolved" => LinksResolved { doc_gen, resolved },
    "open-failed" => OpenFailed { target, error },
    "git-diff" => GitDiff { doc_gen, untracked, total, unstaged },
    "images-probed" => ImagesProbed { doc_gen, probes },
    "editor-exited" => EditorExited { status },
    "file-changed" => FileChanged { path, mtime },
    "config-read" => ConfigRead { path, text },
    "config-written" => ConfigWritten { path },
    "config-failed" => ConfigFailed { error },
    "highlighted" => Highlighted { doc_gen, block, lines, spans },
    "image-ready" => ImageReady { path, mtime, size, image },
}

// --- field values -----------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Value {
    Int(i64),
    Size(u16, u16),
    Ident(String),
    Str(String),
    Blob(Vec<u8>),
}

trait Field: Sized {
    fn write(&self, key: &str, line: &mut String, blobs: &mut Vec<u8>);
    fn read(value: Value) -> Option<Self>;

    /// The value of a field the line leaves out, if it may.
    fn absent() -> Option<Self> {
        None
    }
}

fn inline(key: &str, line: &mut String, value: impl Display) {
    let _ = write!(line, " {key}={value}");
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

macro_rules! int_fields {
    ($($t:ty),*) => {$(
        impl Field for $t {
            fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
                inline(key, line, self);
            }

            fn read(value: Value) -> Option<Self> {
                match value {
                    Value::Int(n) => <$t>::try_from(n).ok(),
                    _ => None,
                }
            }
        }
    )*};
}

int_fields!(i16, i32, u16, u32, u64);

/// Computed results are never written: replay recomputes them (ADR-19).
impl<T> Field for Computed<T> {
    fn write(&self, _: &str, _: &mut String, _: &mut Vec<u8>) {}

    fn read(_: Value) -> Option<Self> {
        None
    }

    fn absent() -> Option<Self> {
        Some(Computed(None))
    }
}

impl Field for bool {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        inline(key, line, self);
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Ident(s) if s == "true" => Some(true),
            Value::Ident(s) if s == "false" => Some(false),
            _ => None,
        }
    }
}

impl Field for Size {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        inline(key, line, format_args!("{}x{}", self.w, self.h));
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Size(w, h) => Some(Size { w, h }),
            _ => None,
        }
    }
}

impl Field for String {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        inline(key, line, quote(self));
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

impl Field for PathBuf {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        inline(key, line, quote(&self.to_string_lossy()));
    }

    fn read(value: Value) -> Option<Self> {
        String::read(value).map(PathBuf::from)
    }
}

impl Field for Arc<str> {
    fn write(&self, key: &str, line: &mut String, blobs: &mut Vec<u8>) {
        let _ = write!(line, " {key}:{}", self.len());
        blobs.extend_from_slice(self.as_bytes());
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Blob(bytes) => String::from_utf8(bytes).ok().map(Arc::from),
            _ => None,
        }
    }
}

/// One line per target: `found "<path>"` or `broken "<reason>"`.
impl Field for Vec<Resolved> {
    fn write(&self, key: &str, line: &mut String, blobs: &mut Vec<u8>) {
        let mut text = String::new();
        for r in self {
            let (kind, value) = match r {
                Resolved::Found(path) => ("found", path.to_string_lossy()),
                Resolved::Broken(why) => ("broken", why.into()),
            };
            let _ = writeln!(text, "{kind} {}", quote(&value));
        }
        let _ = write!(line, " {key}:{}", text.len());
        blobs.extend_from_slice(text.as_bytes());
    }

    fn read(value: Value) -> Option<Self> {
        let Value::Blob(bytes) = value else {
            return None;
        };
        let text = String::from_utf8(bytes).ok()?;
        text.lines()
            .map(|l| {
                let (kind, rest) = l.split_once(' ')?;
                let value = left(quoted(), empty()).parse(rest).result?;
                match kind {
                    "found" => Some(Resolved::Found(value.into())),
                    "broken" => Some(Resolved::Broken(value)),
                    _ => None,
                }
            })
            .collect()
    }
}

/// One line per image: `image <W>x<H> mtime=<ms|none>` or `failed "<reason>"`.
impl Field for Vec<Probe> {
    fn write(&self, key: &str, line: &mut String, blobs: &mut Vec<u8>) {
        let mut text = String::new();
        for p in self {
            let _ = match p {
                Probe::Image { w, h, mtime } => {
                    let mtime = mtime.map_or_else(|| "none".to_owned(), |m| m.to_string());
                    writeln!(text, "image {w}x{h} mtime={mtime}")
                }
                Probe::Failed(why) => writeln!(text, "failed {}", quote(why)),
            };
        }
        let _ = write!(line, " {key}:{}", text.len());
        blobs.extend_from_slice(text.as_bytes());
    }

    fn read(value: Value) -> Option<Self> {
        let Value::Blob(bytes) = value else {
            return None;
        };
        let text = String::from_utf8(bytes).ok()?;
        text.lines()
            .map(|l| match l.split_once(' ')? {
                ("image", rest) => {
                    let (size, mtime) = rest.split_once(" mtime=")?;
                    let (w, h) = size.split_once('x')?;
                    Some(Probe::Image {
                        w: w.parse().ok()?,
                        h: h.parse().ok()?,
                        mtime: match mtime {
                            "none" => None,
                            m => Some(m.parse().ok()?),
                        },
                    })
                }
                ("failed", rest) => left(quoted(), empty())
                    .parse(rest)
                    .result
                    .map(Probe::Failed),
                _ => None,
            })
            .collect()
    }
}

impl Field for Proto {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        inline(key, line, self.name());
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Ident(s) => Proto::parse(&s),
            _ => None,
        }
    }
}

impl Field for Option<u64> {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        match self {
            Some(n) => inline(key, line, n),
            None => inline(key, line, "none"),
        }
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Int(n) => u64::try_from(n).ok().map(Some),
            Value::Ident(s) if s == "none" => Some(None),
            _ => None,
        }
    }
}

/// Left out when `None`: a partial highlighting's line count (O-05), which older recordings don't
/// have.
impl Field for Option<u32> {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        if let Some(n) = self {
            inline(key, line, n);
        }
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Int(n) => u32::try_from(n).ok().map(Some),
            _ => None,
        }
    }

    fn absent() -> Option<Self> {
        Some(None)
    }
}

impl Field for Dir {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        inline(key, line, dir_name(*self));
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Ident(s) if s == "next" => Some(Dir::Next),
            Value::Ident(s) if s == "prev" => Some(Dir::Prev),
            _ => None,
        }
    }
}

fn dir_name(dir: Dir) -> &'static str {
    match dir {
        Dir::Next => "next",
        Dir::Prev => "prev",
    }
}

impl Field for ScrollBy {
    fn write(&self, key: &str, line: &mut String, _: &mut Vec<u8>) {
        match self {
            ScrollBy::Rows(n) => inline(key, line, n),
            ScrollBy::HalfPage(Dir::Next) => inline(key, line, "half-down"),
            ScrollBy::HalfPage(Dir::Prev) => inline(key, line, "half-up"),
            ScrollBy::Page(Dir::Next) => inline(key, line, "page-down"),
            ScrollBy::Page(Dir::Prev) => inline(key, line, "page-up"),
            ScrollBy::Top => inline(key, line, "top"),
            ScrollBy::Bottom => inline(key, line, "bottom"),
        }
    }

    fn read(value: Value) -> Option<Self> {
        match value {
            Value::Int(n) => i32::try_from(n).ok().map(ScrollBy::Rows),
            Value::Ident(s) => match s.as_str() {
                "half-down" => Some(ScrollBy::HalfPage(Dir::Next)),
                "half-up" => Some(ScrollBy::HalfPage(Dir::Prev)),
                "page-down" => Some(ScrollBy::Page(Dir::Next)),
                "page-up" => Some(ScrollBy::Page(Dir::Prev)),
                "top" => Some(ScrollBy::Top),
                "bottom" => Some(ScrollBy::Bottom),
                _ => None,
            },
            _ => None,
        }
    }
}

struct Fields(Vec<(String, Value)>);

impl Fields {
    fn take<T: Field>(&mut self, key: &str) -> Result<T, String> {
        let Some(i) = self.0.iter().position(|(k, _)| k == key) else {
            return T::absent().ok_or_else(|| format!("missing field `{key}`"));
        };
        let (_, value) = self.0.swap_remove(i);
        T::read(value).ok_or_else(|| format!("bad value for `{key}`"))
    }
}

// --- the line grammar -------------------------------------------------------------------------

enum Raw<'a> {
    Int(i64),
    Size(u16, u16),
    Ident(&'a str),
    Str(String),
    Blob(usize),
}

fn ident<'a>() -> impl StrParser<'a, &'a str> {
    not_empty(item_while(|c: char| {
        c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
    }))
}

fn quoted<'a>() -> impl StrParser<'a, String> {
    let escaped = right(
        skip('\\'),
        map_if(item(), |c: char| match c {
            'n' => Some('\n'),
            '"' => Some('"'),
            '\\' => Some('\\'),
            _ => None,
        }),
    );
    let plain = item_if(|c: char| c != '"' && c != '\\');
    let body = fold(
        or(escaped, plain),
        String::new,
        |s: &mut String, c: char| s.push(c),
        true,
        no_separator(),
    );
    middle(skip('"'), body, skip('"'))
}

/// A value ends at a space or the end of the line.
fn end_of_value<'a>() -> impl StrParser<'a, ()> {
    peek(or_diff(skip(' '), empty()))
}

fn value<'a>() -> impl StrParser<'a, Raw<'a>> {
    let size = attempt(left(
        tuplify!(left(integer(), skip('x')), integer()),
        end_of_value(),
    ));
    let int = attempt(left(integer(), end_of_value()));
    or!(
        map(quoted(), Raw::Str),
        map(size, |(w, h)| Raw::Size(w, h)),
        map(int, Raw::Int),
        map(ident(), Raw::Ident),
    )
}

fn field<'a>() -> impl StrParser<'a, (&'a str, Raw<'a>)> {
    tuplify!(
        ident(),
        or(
            right(skip('='), value()),
            right(skip(':'), map(integer(), Raw::Blob))
        )
    )
}

type Line<'a> = (u64, &'a str, Vec<(&'a str, Raw<'a>)>);

fn line<'a>() -> impl StrParser<'a, Line<'a>> {
    let fields = fold(
        right(skip(' '), field()),
        Vec::new,
        |v: &mut Vec<_>, f| v.push(f),
        true,
        no_separator(),
    );
    left(
        tuplify!(
            right(skip('+'), integer()),
            right(skip(' '), ident()),
            fields
        ),
        empty(),
    )
}

// --- reading and writing ----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Milliseconds since the session started.
    pub at_ms: u64,
    pub msg: Msg,
}

/// Append one entry to `out`.
pub fn encode_entry(at_ms: u64, msg: &Msg, out: &mut Vec<u8>) {
    let mut line = format!("+{at_ms} ");
    let mut blobs = Vec::new();
    encode(msg, &mut line, &mut blobs);
    line.push('\n');
    out.extend_from_slice(line.as_bytes());
    if !blobs.is_empty() {
        out.extend_from_slice(&blobs);
        out.push(b'\n');
    }
}

fn take_line<'a>(bytes: &'a [u8], pos: &mut usize) -> &'a [u8] {
    let rest = &bytes[*pos..];
    let len = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    *pos += (len + 1).min(rest.len());
    &rest[..len]
}

/// Parse a recording.
pub fn parse(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let mut pos = 0;
    let header = take_line(bytes, &mut pos);
    if header != HEADER.as_bytes() {
        return Err(format!(
            "not a recording: expected `{HEADER}` on the first line"
        ));
    }
    let mut entries = Vec::new();
    while pos < bytes.len() {
        let raw = take_line(bytes, &mut pos);
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let n = entries.len() + 1;
        let text = std::str::from_utf8(raw).map_err(|_| format!("entry {n}: not UTF-8"))?;
        let (at_ms, name, fields) = line()
            .parse(text)
            .result
            .ok_or_else(|| format!("entry {n}: cannot parse `{text}`"))?;
        let mut values = Vec::with_capacity(fields.len());
        let mut blobs = false;
        for (key, raw) in fields {
            let value = match raw {
                Raw::Blob(len) => {
                    blobs = true;
                    let end = pos
                        .checked_add(len)
                        .filter(|&end| end <= bytes.len())
                        .ok_or_else(|| format!("entry {n}: `{key}` runs past the end"))?;
                    let blob = bytes[pos..end].to_vec();
                    pos = end;
                    Value::Blob(blob)
                }
                Raw::Int(i) => Value::Int(i),
                Raw::Size(w, h) => Value::Size(w, h),
                Raw::Ident(s) => Value::Ident(s.to_owned()),
                Raw::Str(s) => Value::Str(s),
            };
            values.push((key.to_owned(), value));
        }
        if blobs && bytes.get(pos) == Some(&b'\n') {
            pos += 1;
        }
        let msg = decode(name, Fields(values)).map_err(|e| format!("entry {n}: {e}"))?;
        entries.push(Entry { at_ms, msg });
    }
    Ok(entries)
}

/// `--record FILE`: logs every message as it reaches `update`.
pub struct Recorder {
    out: BufWriter<File>,
    start: Instant,
    buf: Vec<u8>,
}

impl Recorder {
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(out, "{HEADER}")?;
        Ok(Self {
            out,
            start: Instant::now(),
            buf: Vec::new(),
        })
    }

    pub fn log(&mut self, msg: &Msg) -> io::Result<()> {
        self.buf.clear();
        encode_entry(self.start.elapsed().as_millis() as u64, msg, &mut self.buf);
        self.out.write_all(&self.buf)?;
        // flushed per message, so a crash keeps everything up to it
        self.out.flush()
    }
}

// --- replay -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct ReplayOptions {
    /// Re-execute commands, skipping recorded world messages (for benchmarking).
    pub live: bool,
    /// Render without a terminal and print the final frame.
    pub headless: bool,
}

/// `--replay FILE`.
pub fn replay(
    path: &Path,
    mut state: State,
    mut effects: Effects,
    opts: ReplayOptions,
) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let entries = parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if opts.headless {
        let frames = replay_headless(&entries, &mut state, &mut effects, opts.live)?;
        if let Some(last) = frames.last() {
            print!("{last}");
        }
        Ok(())
    } else {
        replay_terminal(&entries, &mut state, &mut effects, opts.live)
    }
}

/// Apply one message, and in live mode the messages its commands produce. Otherwise only a
/// highlighting marker's job runs, inline, so its frame shows the highlighting (ADR-19).
fn step(state: &mut State, msg: Msg, effects: &mut Effects, live: bool) {
    let mut queue = VecDeque::from([msg]);
    while let Some(msg) = queue.pop_front() {
        let marker = matches!(msg, Msg::Highlighted { .. });
        for cmd in app::update(state, msg) {
            if live || marker {
                queue.extend(effects.run(cmd));
            }
        }
    }
}

/// Replay without a terminal, returning the frame after every message.
pub fn replay_headless(
    entries: &[Entry],
    state: &mut State,
    effects: &mut Effects,
    live: bool,
) -> Result<Vec<String>, String> {
    let mut grid = Grid::empty(Rect::new(0, 0, state.size.w, state.size.h));
    let mut frames = Vec::new();
    for entry in entries {
        if live && entry.msg.is_result() {
            continue;
        }
        step(state, entry.msg.clone(), effects, live);
        if state.quit {
            break;
        }
        let area = Rect::new(0, 0, state.size.w, state.size.h);
        if grid.area() == area {
            grid.reset();
        } else {
            grid.resize(area);
        }
        ui::view(state, &mut grid);
        frames.push(frame_text(&grid));
    }
    Ok(frames)
}

fn replay_terminal(
    entries: &[Entry],
    state: &mut State,
    effects: &mut Effects,
    live: bool,
) -> Result<(), String> {
    let mut terminal = app::init_terminal().map_err(|e| format!("terminal: {e}"))?;
    let start = Instant::now();
    let mut result = Ok(());
    for entry in entries {
        if live && entry.msg.is_result() {
            continue;
        }
        if let Some(wait) = Duration::from_millis(entry.at_ms).checked_sub(start.elapsed()) {
            thread::sleep(wait);
        }
        step(state, entry.msg.clone(), effects, live);
        if state.quit {
            break;
        }
        if let Err(e) = app::draw_frame(&mut terminal, state) {
            result = Err(e.to_string());
            break;
        }
    }
    app::restore_terminal();
    result
}

/// A frame as plain text, one line per row, trailing blanks trimmed.
pub fn frame_text(grid: &Grid) -> String {
    let area = grid.area();
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        let mut skip = 0;
        for x in area.left()..area.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = grid.symbol((x, y));
            line.push_str(symbol);
            skip = symbol.width().saturating_sub(1);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Config;
    use crate::theme::Theme;

    fn every_message() -> Vec<Msg> {
        vec![
            Msg::Init {
                size: Size { w: 120, h: 40 },
            },
            Msg::Graphics {
                proto: Proto::Kitty,
                cell: Size { w: 9, h: 18 },
            },
            Msg::Resize {
                size: Size { w: 100, h: 30 },
            },
            Msg::Open {
                path: "notes/a \"quoted\" \\ name.md".into(),
            },
            Msg::Scroll {
                by: ScrollBy::Rows(-3),
            },
            Msg::Scroll {
                by: ScrollBy::HalfPage(Dir::Next),
            },
            Msg::Scroll {
                by: ScrollBy::Page(Dir::Prev),
            },
            Msg::Scroll { by: ScrollBy::Top },
            Msg::Scroll {
                by: ScrollBy::Bottom,
            },
            Msg::JumpHeading { dir: Dir::Prev },
            Msg::ToggleTableMode,
            Msg::SortOpen,
            Msg::SortTable { dir: Dir::Prev },
            Msg::ToggleCodeWrap,
            Msg::ToggleLineNumbers,
            Msg::ScrollCodeX { by: -8 },
            Msg::FocusLink { dir: Dir::Next },
            Msg::Follow,
            Msg::Back,
            Msg::Forward,
            Msg::OpenOutline,
            Msg::OpenLinks,
            Msg::OutlineFilter {
                text: "p4 \"links\"".into(),
            },
            Msg::OutlineMove { by: -3 },
            Msg::OutlinePick,
            Msg::OutlineClose,
            Msg::SearchOpen,
            Msg::SearchInput {
                pattern: "foo\\s+bar".into(),
            },
            Msg::SearchSubmit,
            Msg::SearchCancel,
            Msg::SearchNext { dir: Dir::Prev },
            Msg::LinksResolved {
                doc_gen: 2,
                resolved: vec![
                    Resolved::Found("notes/a \"b\".md".into()),
                    Resolved::Broken("no such file: x.md".into()),
                ],
            },
            Msg::LinksResolved {
                doc_gen: 3,
                resolved: Vec::new(),
            },
            Msg::JumpChange { dir: Dir::Next },
            Msg::Reload,
            Msg::OpenThemes,
            Msg::ThemesTone { dir: Dir::Next },
            Msg::Help,
            Msg::HelpScroll { by: -10 },
            Msg::HelpClose,
            Msg::Edit,
            Msg::Yank,
            Msg::SaveConfig,
            Msg::ConfigRead {
                path: "/home/u/.config/ada/config.toml".into(),
                text: "# options\ntables = \"grid\"\n".into(),
            },
            Msg::SaveConfirm,
            Msg::SaveCancel,
            Msg::ConfigWritten {
                path: "/home/u/.config/ada/config.toml".into(),
            },
            Msg::ConfigFailed {
                error: "can't write config.toml: Permission denied".into(),
            },
            Msg::EditorExited { status: -1 },
            Msg::FileChanged {
                path: "notes/a.md".into(),
                mtime: Some(1_760_535_362_999),
            },
            Msg::GitDiff {
                doc_gen: 4,
                untracked: false,
                total: "@@ -1 +1 @@\n-a\n+b\n".into(),
                unstaged: "".into(),
            },
            Msg::OpenFailed {
                target: "x.png".into(),
                error: "No such file".into(),
            },
            Msg::Highlighted {
                doc_gen: 3,
                block: 12,
                lines: None,
                spans: Computed(None),
            },
            Msg::Highlighted {
                doc_gen: 3,
                block: 13,
                lines: Some(512),
                spans: Computed(None),
            },
            Msg::ImagesProbed {
                doc_gen: 3,
                probes: vec![
                    Probe::Image {
                        w: 686,
                        h: 811,
                        mtime: Some(1_760_535_362_123),
                    },
                    Probe::Image {
                        w: 1,
                        h: 2,
                        mtime: None,
                    },
                    Probe::Failed("The image format \"Gif\" is not supported".into()),
                ],
            },
            Msg::ImageReady {
                path: "img/cat.png".into(),
                mtime: None,
                size: Size { w: 40, h: 12 },
                image: Computed(None),
            },
            Msg::DocLoaded {
                path: "a.md".into(),
                mtime: Some(1_760_535_362_123),
                text: "# Title\n\nno trailing newline".into(),
                doc: Computed(None),
            },
            Msg::DocLoaded {
                path: "-".into(),
                mtime: None,
                text: "".into(),
                doc: Computed(None),
            },
            Msg::DocFailed {
                path: "gone.md".into(),
                error: "No such file\nor directory".into(),
            },
            Msg::Quit,
        ]
    }

    #[test]
    fn round_trip_covers_every_variant() {
        let msgs = every_message();
        let mut bytes = format!("{HEADER}\n").into_bytes();
        for (i, msg) in msgs.iter().enumerate() {
            encode_entry(i as u64 * 7, msg, &mut bytes);
        }
        let entries = parse(&bytes).unwrap();
        let back: Vec<_> = entries.iter().map(|e| e.msg.clone()).collect();
        assert_eq!(back, msgs);
        assert_eq!(entries[3].at_ms, 21);

        let mut names: Vec<_> = msgs
            .iter()
            .map(|m| {
                let mut line = String::new();
                encode(m, &mut line, &mut Vec::new());
                line
            })
            .map(|l| l.split(' ').next().unwrap_or_default().to_owned())
            .collect();
        names.sort();
        names.dedup();
        let mut expected = NAMES.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[test]
    fn reads_the_documented_format() {
        let text = "ada-rec 1\n+0 init size=120x40\n+3 doc-loaded path=\"notes/a.md\" mtime=1760535362123 text:7\n# a\nb\nc\n+412 scroll by=3\n+500 scroll by=page-down\n+980 resize size=100x40\n\n+5000 quit\n";
        let entries = parse(text.as_bytes()).unwrap();
        assert_eq!(entries.len(), 6);
        assert_eq!(
            entries[1].msg,
            Msg::DocLoaded {
                path: "notes/a.md".into(),
                mtime: Some(1_760_535_362_123),
                text: "# a\nb\nc".into(),
                doc: Computed(None),
            }
        );
        assert_eq!(
            entries[5],
            Entry {
                at_ms: 5000,
                msg: Msg::Quit
            }
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(b"ada-rec 2\n").is_err());
        assert!(parse(b"ada-rec 1\n+0 dance\n").is_err());
        assert!(parse(b"ada-rec 1\n+0 scroll by=sideways\n").is_err());
        assert!(parse(b"ada-rec 1\n+0 doc-loaded path=\"x\" mtime=none text:99\nshort").is_err());
        assert!(parse(b"ada-rec 1\n+x quit\n").is_err());
    }

    /// A marker's job runs inline, so the frame after it is highlighted (ADR-19). Frame text
    /// doesn't show colors, so this looks at the state.
    #[test]
    fn replayed_markers_highlight_before_the_next_frame() {
        let mut state = State::new(Theme::builtin(), Config::default());
        let mut effects = Effects::new(None);
        let load = [
            Msg::Init {
                size: Size { w: 80, h: 10 },
            },
            Msg::DocLoaded {
                path: "a.md".into(),
                mtime: None,
                text: "```rust\nfn main() {}\n```\n".into(),
                doc: Computed(None),
            },
        ]
        .map(|msg| Entry { at_ms: 0, msg });
        replay_headless(&load, &mut state, &mut effects, false).unwrap();
        let doc = state.doc.as_ref().unwrap();
        let key = crate::highlight::key(doc.doc.code_blocks()[0]).unwrap();
        assert!(state.highlights.get(key).is_none());
        let marker = Entry {
            at_ms: 0,
            msg: Msg::Highlighted {
                doc_gen: doc.generation,
                block: 0,
                lines: None,
                spans: Computed(None),
            },
        };
        replay_headless(&[marker], &mut state, &mut effects, false).unwrap();
        let spans = state
            .highlights
            .get(key)
            .expect("highlighted by the replay");
        assert!(!spans.line(0).is_empty());
    }

    #[test]
    fn checked_in_sessions_replay_identically() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sessions");
        let mut checked = 0;
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rec") {
                continue;
            }
            let entries = parse(&fs::read(&path).unwrap()).unwrap();
            let run = || {
                let mut state = State::new(Theme::builtin(), Config::default());
                replay_headless(&entries, &mut state, &mut Effects::new(None), false).unwrap()
            };
            let frames = run();
            assert!(!frames.is_empty(), "{}", path.display());
            assert_eq!(frames, run(), "{} replays differently", path.display());
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            insta::assert_snapshot!(name, frames.last().unwrap());
            checked += 1;
        }
        assert!(checked > 0);
    }
}
