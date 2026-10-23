//! `--print` of a big log, JSON or text file a piece at a time (O-52). A whole print holds the
//! file, its layout and its highlighting at once, ~6× the file at the peak (476 MiB for an 80 MB
//! log). Here a reader cuts the input into pieces of whole lines, workers highlight, lay out and
//! paint each piece as a document of its own, and the caller's thread writes them in order. The
//! output is the whole print's, byte for byte: a piece's rows are the rows its lines get in the
//! whole layout, numbered for the whole file, with the code block's top row only in the first
//! piece and its bottom row only in the last.
//!
//! Only lexers whose lines don't depend on each other stream ([`Lexer`]); JSON's one state, a
//! `/* */` comment left open, the reader carries from piece to piece, scanning only the lines
//! that can open or close one.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use crate::app;
use crate::doc::{self, BlockKind};
use crate::filetype::{self, FileType};
use crate::highlight::{Highlights, Lexer, SyntaxTheme};
use crate::layout::{self, Layout, LayoutParams, LineNumbers};
use crate::theme::Theme;
use crate::ui::Painter;

use super::{end_line, start_line};

/// Inputs smaller than this print whole: streaming them saves little.
const FIRST: usize = 1 << 20;

/// How much of a file is read to tell what it holds: more than [`crate::sniff`] looks at, and
/// little enough that the whole print, if it comes to that, finds malloc as it would have.
const SAMPLE: usize = 64 << 10;

/// About how much of the input a piece holds; a piece ends on a newline, so a longer line makes
/// a longer piece.
const PIECE: usize = 128 << 10;

/// The most lines a piece holds: a short line can print many times its size, a row of code
/// background and all.
const LINES: usize = 1024;

/// Where the reader cuts: [`PIECE`] and [`LINES`], or smaller for the fuzzer (F-07), whose inputs
/// stay far under a piece.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// About how many bytes a piece holds.
    pub piece: usize,
    /// The most lines a piece holds.
    pub lines: usize,
}

impl Limits {
    const DEFAULT: Limits = Limits {
        piece: PIECE,
        lines: LINES,
    };
}

/// How much is read at a time.
const READ: usize = 64 << 10;

/// The start of a file or stdin, and whether it prints a piece at a time.
pub enum Start {
    Stream(Stream),
    /// Print it whole, with what was read of stdin so far, which can't be read again.
    Whole(Vec<u8>),
}

/// An input that prints a piece at a time: what was read of it so far, and the rest.
pub struct Stream {
    first: Vec<u8>,
    rest: Box<dyn Read + Send>,
    lang: Option<&'static str>,
    /// The file's line count, which sets the width of line numbers; 0 without them.
    lines: u32,
    limits: Limits,
}

/// Read the start of `path` (`-` for stdin) and tell whether it streams: a log, JSON or text
/// file of 1 MiB (`FIRST`) or more, shown as `forced` or as what it holds. With line numbers
/// (`numbered`) the lines are counted first, which stdin can't do.
pub fn start(path: &Path, forced: Option<FileType>, numbered: bool) -> Start {
    let stdin = path == Path::new("-");
    if stdin && numbered {
        return Start::Whole(Vec::new());
    }
    let by_path = FileType::by_path(path);
    if matches!(by_path, Some(FileType::Markdown | FileType::Image)) {
        return Start::Whole(Vec::new());
    }
    // a file's size tells whether it streams, and only its start is read to tell what it is;
    // stdin is read as far as that size
    let (mut rest, want): (Box<dyn Read + Send>, usize) = if stdin {
        (Box::new(io::stdin()), FIRST)
    } else {
        let big = fs::metadata(path).is_ok_and(|m| m.len() >= FIRST as u64);
        match File::open(path) {
            Ok(file) if big => (Box::new(file), SAMPLE),
            _ => return Start::Whole(Vec::new()),
        }
    };
    let whole = |first: Vec<u8>| Start::Whole(if stdin { first } else { Vec::new() });
    let mut first = Vec::new();
    let Ok(full) = fill(&mut rest, &mut first, want) else {
        return whole(first);
    };
    // a piped image is shown whole, as an image
    let image = stdin && forced.is_none() && filetype::image_ext(&first).is_some();
    if !full || image || filetype::is_binary(&first) || !first.contains(&b'\n') {
        return whole(first);
    }
    // the whole print's type: sniffing looks only at the first few KB and the first line
    let filetype = forced.unwrap_or_else(|| FileType::of(path, &String::from_utf8_lossy(&first)));
    let FileType::Text { lang } = filetype else {
        return whole(first);
    };
    if !Lexer::exists(lang) {
        return whole(first);
    }
    let lines = if numbered {
        match File::open(path).and_then(count_lines) {
            Ok(lines) => lines,
            Err(_) => return whole(first),
        }
    } else {
        0
    };
    Start::Stream(Stream {
        first,
        rest,
        lang,
        lines,
        limits: Limits::DEFAULT,
    })
}

/// Print `bytes`, a file of `lang`, as [`start`] would stream it, but cut at `limits`, `first`
/// bytes of it read before the rest, and line numbers counted as the whole print counts them. The fuzzer's way in (F-07): it checks the
/// output against [`super::render`] of the whole file. `lang` must be one a lexer highlights
/// (`None`, `"log"` or `"JSON"`).
#[doc(hidden)]
pub fn write_bytes(
    bytes: &[u8],
    lang: Option<&'static str>,
    first: usize,
    limits: Limits,
    theme: &Theme,
    params: LayoutParams,
    w: &mut impl Write,
) -> io::Result<()> {
    assert!(Lexer::exists(lang), "no lexer streams {lang:?}");
    let first = first.min(bytes.len());
    Stream {
        first: bytes[..first].to_vec(),
        rest: Box::new(io::Cursor::new(bytes[first..].to_vec())),
        lang,
        lines: count_lines(bytes)?,
        limits: Limits {
            piece: limits.piece.max(1),
            lines: limits.lines.max(1),
        },
    }
    .write(theme, params, w)
}

/// Read into `buf` until it holds `n` bytes or the input ends. Whether it holds them.
fn fill(input: &mut impl Read, buf: &mut Vec<u8>, n: usize) -> io::Result<bool> {
    while buf.len() < n {
        if read_more(input, buf)? == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Read up to [`READ`] more bytes onto `buf`; 0 at the end.
fn read_more(input: &mut impl Read, buf: &mut Vec<u8>) -> io::Result<usize> {
    let start = buf.len();
    buf.resize(start + READ, 0);
    let read = loop {
        match input.read(&mut buf[start..]) {
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            other => break other,
        }
    };
    buf.truncate(start + *read.as_ref().unwrap_or(&0));
    read
}

/// A file's lines as the whole document counts them ([`doc::Doc::line_count`]): one more than
/// its newlines, but for one that ends it.
fn count_lines(mut file: impl Read) -> io::Result<u32> {
    let mut buf = vec![0; 256 << 10];
    let (mut newlines, mut last) = (0usize, None);
    loop {
        let n = match file.read(&mut buf) {
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            other => other?,
        };
        if n == 0 {
            break;
        }
        newlines += buf[..n].iter().filter(|&&b| b == b'\n').count();
        last = Some(buf[n - 1]);
    }
    Ok((1 + newlines - usize::from(last == Some(b'\n'))) as u32)
}

/// Where the next piece of `buf` ends: on the newline that ends its `lines`th line, or else the
/// last newline in its first `piece` bytes, or else the first one after them (`scanned` bytes
/// past them are known to hold none). Never on the first byte, which would make a piece of a
/// lone empty line. `None` until `buf` holds a piece.
fn cut(buf: &[u8], scanned: usize, limits: Limits) -> Option<usize> {
    let head = &buf[..buf.len().min(limits.piece)];
    let mut last = None;
    let newlines = head.iter().enumerate().filter(|&(_, &b)| b == b'\n');
    for (n, (i, _)) in newlines.enumerate() {
        // at the first byte only with `lines` 1, which the fuzzer sets: take the next newline
        if n + 1 >= limits.lines && i > 0 {
            return Some(i);
        }
        last = Some(i);
    }
    if buf.len() < limits.piece {
        return None;
    }
    if let Some(c) = last.filter(|&c| c > 0) {
        return Some(c);
    }
    let from = scanned.max(limits.piece);
    buf[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|c| from + c)
}

/// A run of whole lines of the input.
struct Piece {
    index: usize,
    text: Arc<str>,
    /// The file's line number of its first line.
    first_line: u32,
    /// Whether it starts or ends the file, and so draws the top or bottom row of the block.
    first: bool,
    last: bool,
    /// Whether it starts inside a JSON comment.
    comment: bool,
}

/// What every piece is painted with.
struct Paint<'a> {
    theme: &'a Theme,
    syntax: SyntaxTheme,
    lexer: Lexer,
    lang: Option<&'static str>,
    params: LayoutParams,
    lines: u32,
}

impl Paint<'_> {
    /// Paint a piece; `spare` is a layout no longer needed, and comes back for the next piece.
    fn piece(&self, piece: &Piece, spare: Layout) -> (String, Layout) {
        let text = Arc::clone(&piece.text);
        let doc = doc::file::code_piece(text, self.lang, piece.first_line, piece.index as u64);
        let BlockKind::Code(code) = &doc.blocks[0].kind else {
            unreachable!("a piece is one code block");
        };
        let (spans, _) = self.lexer.highlight(code, &self.syntax, piece.comment);
        let mut highlights = Highlights::default();
        if let Some(spans) = spans {
            highlights.insert(Arc::new(spans));
        }
        let layout =
            layout::relayout_piece(&doc, &self.theme.glyphs, self.params, self.lines, spare);
        let painter = Painter {
            theme: self.theme,
            layout: &layout,
            highlights: &highlights,
            code_x: 0,
            links: &[],
        };
        // the block's top and bottom rows, where the piece doesn't start or end the file
        let rows = layout.rows.len();
        let from = usize::from(!piece.first);
        let to = rows - usize::from(!piece.last);
        let mut out = String::with_capacity(piece.text.len() * 2);
        let mut scratch = String::new();
        for (r, row) in layout.rows.iter().enumerate().take(to).skip(from) {
            let line = start_line(&painter, r, row, &mut out, &mut scratch);
            end_line(&line, &mut out);
        }
        (out, layout)
    }
}

impl Stream {
    /// Print to stdout. A closed pipe (fzf moving on) is not an error.
    pub fn run(self, theme: &Theme, params: LayoutParams) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        match self
            .write(theme, params, &mut stdout)
            .and_then(|()| stdout.flush())
        {
            Err(e) if e.kind() == ErrorKind::BrokenPipe => Ok(()),
            result => result,
        }
    }

    fn write(self, theme: &Theme, params: LayoutParams, w: &mut impl Write) -> io::Result<()> {
        let syntax = SyntaxTheme::new(&theme.syntax);
        let Some(lexer) = Lexer::new(self.lang, &syntax) else {
            unreachable!("`start` streams only what a lexer highlights");
        };
        let params = FileType::Text { lang: self.lang }.params(params);
        let paint = Paint {
            theme,
            syntax,
            lexer,
            lang: self.lang,
            params,
            lines: if params.line_numbers == LineNumbers::Off {
                0
            } else {
                self.lines
            },
        };
        let workers = thread::available_parallelism().map_or(1, |n| n.get().min(8));
        app::arenas(workers);
        // pieces read and not yet written: bounds the memory
        let in_flight = 2 * workers;
        let (
            Stream {
                first,
                rest,
                limits,
                ..
            },
            paint,
        ) = (self, &paint);
        let (jobs_tx, jobs) = mpsc::sync_channel::<Piece>(in_flight);
        let jobs = Mutex::new(jobs);
        let (done_tx, done) = mpsc::channel();
        thread::scope(|s| {
            // dropped when the writing stops, or panics, so that a reader waiting for a token
            // stops too
            let (tokens_tx, tokens) = mpsc::sync_channel(in_flight);
            for _ in 0..in_flight {
                let _ = tokens_tx.send(());
            }
            let lexer = &paint.lexer;
            let reader =
                s.spawn(move || read_pieces(rest, first, limits, lexer, &tokens, &jobs_tx));
            for _ in 0..workers {
                let (jobs, done_tx) = (&jobs, done_tx.clone());
                s.spawn(move || {
                    let mut spare = Layout::empty(paint.params);
                    loop {
                        let job = jobs.lock().unwrap_or_else(PoisonError::into_inner).recv();
                        let Ok(piece) = job else {
                            return;
                        };
                        // a panic stops the writing, which would wait for this piece, then goes
                        // on as it would in a whole print
                        let painted =
                            panic::catch_unwind(AssertUnwindSafe(|| paint.piece(&piece, spare)));
                        let (out, layout) = match painted {
                            Ok(painted) => painted,
                            Err(panic) => {
                                let _ = done_tx.send((piece.index, None));
                                panic::resume_unwind(panic);
                            }
                        };
                        spare = layout;
                        if done_tx.send((piece.index, Some(out))).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(done_tx);
            let written = write_in_order(done, &tokens_tx, w);
            drop(tokens_tx);
            let read = reader.join().expect("reader thread");
            written.and(read)
        })
    }
}

/// Write the painted pieces as they come back, in order, handing back a token for each to let
/// the reader read on. A piece that comes back unpainted stops the writing.
fn write_in_order(
    done: Receiver<(usize, Option<String>)>,
    tokens: &SyncSender<()>,
    w: &mut impl Write,
) -> io::Result<()> {
    let mut waiting = BTreeMap::new();
    let mut next = 0;
    for (index, out) in done {
        waiting.insert(index, out);
        while let Some(out) = waiting.remove(&next) {
            let Some(out) = out else {
                return Err(io::Error::other("a piece of the file failed to print"));
            };
            w.write_all(out.as_bytes())?;
            next += 1;
            let _ = tokens.send(());
        }
    }
    Ok(())
}

/// Cut the input, `buf` read of it so far and `input` the rest, into pieces of whole lines, a
/// piece for each token. A piece ends on a newline and is never the lone newline of an empty
/// line, which [`doc::code_lines`] would read as no line at all; at the end, the file's last
/// empty line joins the piece before it for the same reason.
fn read_pieces(
    mut input: impl Read,
    mut buf: Vec<u8>,
    limits: Limits,
    lexer: &Lexer,
    tokens: &Receiver<()>,
    jobs: &SyncSender<Piece>,
) -> io::Result<()> {
    let send = |piece: Piece| tokens.recv().is_ok() && jobs.send(piece).is_ok();
    let (mut index, mut line, mut comment) = (0, 1, false);
    let mut piece = |text: &[u8], last: bool| {
        let text: Arc<str> = match std::str::from_utf8(text) {
            Ok(text) => Arc::from(text),
            Err(_) => Arc::from(String::from_utf8_lossy(text)),
        };
        let p = Piece {
            index,
            first_line: line,
            first: index == 0,
            last,
            comment,
            text,
        };
        index += 1;
        line += p.text.bytes().filter(|&b| b == b'\n').count() as u32;
        comment = lexer.comment_after(&p.text, comment);
        p
    };
    let mut pending: Option<Piece> = None;
    // bytes of `buf` past `limits.piece` known to hold no newline
    let mut scanned = 0;
    let mut eof = false;
    loop {
        let cut = loop {
            if let Some(c) = cut(&buf, scanned, limits) {
                break Some(c);
            }
            if buf.len() >= limits.piece {
                scanned = buf.len();
            }
            if eof {
                break None;
            }
            eof = read_more(&mut input, &mut buf)? == 0;
        };
        let Some(cut) = cut else {
            break;
        };
        let p = piece(&buf[..=cut], false);
        buf.drain(..=cut);
        scanned = 0;
        if let Some(p) = pending.replace(p)
            && !send(p)
        {
            return Ok(());
        }
    }
    // what's left after the last cut: nothing, the last empty line, or the last piece
    let last = match pending {
        Some(mut p) if buf.is_empty() || buf == b"\n" => {
            if !buf.is_empty() {
                p.text = format!("{}\n", p.text).into();
            }
            p.last = true;
            p
        }
        Some(p) => {
            if !send(p) {
                return Ok(());
            }
            piece(&buf, true)
        }
        None => piece(&buf, true),
    };
    send(last);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::io::Cursor;

    use super::*;
    use crate::print::{Options, render};

    fn params(width: u16, line_numbers: LineNumbers) -> LayoutParams {
        LayoutParams {
            width,
            max_width: None,
            table_mode: Default::default(),
            code_wrap: true,
            line_numbers,
            glyphs_gen: 0,
            images: None,
        }
    }

    /// Print `bytes` in `lang` whole and a piece at a time, and check that the two agree.
    fn same(bytes: &[u8], lang: Option<&'static str>, width: u16, numbers: LineNumbers) {
        let theme = Theme::builtin();
        let filetype = FileType::Text { lang };
        let text = String::from_utf8_lossy(bytes);
        let doc = filetype.parse(Path::new("x"), &text);
        let params = params(width, numbers);
        let whole = render(&doc, &theme, filetype.params(params), Options::default());
        let lines = count_lines(bytes).unwrap();
        assert_eq!(lines, doc.line_count());
        let split = bytes.len().min(SAMPLE);
        let stream = Stream {
            first: bytes[..split].to_vec(),
            rest: Box::new(Cursor::new(bytes[split..].to_vec())),
            lang,
            lines,
            limits: Limits::DEFAULT,
        };
        let mut out = Vec::new();
        stream.write(&theme, params, &mut out).unwrap();
        let streamed = String::from_utf8(out).unwrap();
        if streamed != whole {
            let (a, b): (Vec<_>, Vec<_>) = (whole.lines().collect(), streamed.lines().collect());
            let at = a
                .iter()
                .zip(&b)
                .position(|(a, b)| a != b)
                .unwrap_or(a.len().min(b.len()));
            panic!(
                "{lang:?}, {} bytes: whole {} lines, streamed {}; first difference at line {at}:\n{:?}\n{:?}",
                bytes.len(),
                a.len(),
                b.len(),
                a.get(at),
                b.get(at),
            );
        }
    }

    /// Journal lines of varied lengths, some with tabs, past several pieces.
    fn log(lines: usize) -> String {
        let mut out = String::new();
        for i in 0..lines {
            let level = ["INFO", "WARN", "ERROR", "debug"][i % 4];
            let pad = "x".repeat(i * 7 % 180);
            let _ = writeln!(
                out,
                "2026-10-19T18:{:02}:{:02}+0200 host unit[{i}]: {level} id=0x{i:x}\tpath=/a/b \"q {pad}\"",
                i / 60 % 60,
                i % 60
            );
        }
        out
    }

    #[test]
    fn a_streamed_log_prints_as_a_whole_one() {
        let text = log(5000);
        let numbers = LineNumbers::Code;
        for ending in ["", "\n", "\n\n", "\n\n\n", "tail without a newline"] {
            same(
                format!("{text}{ending}").as_bytes(),
                Some("log"),
                100,
                numbers,
            );
        }
        same(
            format!("\n{text}").as_bytes(),
            Some("log"),
            100,
            LineNumbers::Off,
        );
        same(
            text.replace('\n', "\r\n").as_bytes(),
            Some("log"),
            100,
            numbers,
        );
        same(text.as_bytes(), Some("log"), 23, numbers);
        // a line longer than a piece, inside and at the end
        let long = "y".repeat(PIECE + 1000);
        let (head, tail) = text.split_at(text.len() / 2);
        let cut = head.rfind('\n').unwrap() + 1;
        let inside = format!("{}{long}\n{}{tail}", &head[..cut], &head[cut..]);
        same(inside.as_bytes(), Some("log"), 100, numbers);
        same(
            format!("{text}{long}\n\n").as_bytes(),
            Some("log"),
            100,
            numbers,
        );
        // invalid UTF-8, read as replacement characters either way
        let mut bad = text.clone().into_bytes();
        for i in (100..bad.len()).step_by(7777) {
            bad[i] = 0xff;
        }
        same(&bad, Some("log"), 100, numbers);
    }

    #[test]
    fn short_lines_make_pieces_of_many_lines() {
        let text = "a line\n\n\n".repeat(LINES * 3);
        same(text.as_bytes(), None, 80, LineNumbers::Code);
        same(format!("{text}\n").as_bytes(), None, 80, LineNumbers::Off);
    }

    #[test]
    fn json_comments_carry_over_pieces() {
        let mut text = String::from("{\n");
        for i in 0..6000 {
            match i % 1500 {
                10 => text.push_str("  /* a comment that runs on\n"),
                // the comment closes LINES lines later, pieces after it opened
                c if c == 10 + LINES + 7 => {
                    text.push_str("  still \"/*\" inside */ \"k\": 1, /* open\n");
                }
                c if c == 10 + LINES + 9 => {
                    text.push_str("  closed */ \"url\": \"http://x/*y\", // /* no\n");
                }
                // a `*/` in a string, but not inside the comment, where it would close it
                c if c > 10 + LINES + 9 => {
                    let _ = writeln!(text, "  \"key{i}\": [{i}, true, null, \"v*/\"],");
                }
                _ => {
                    let _ = writeln!(text, "  \"key{i}\": [{i}, true, null],");
                }
            }
        }
        text.push_str("}\n");
        same(text.as_bytes(), Some("JSON"), 100, LineNumbers::Code);
        same(text.as_bytes(), Some("JSON"), 31, LineNumbers::Off);
    }
}
