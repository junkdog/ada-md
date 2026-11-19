//! A diff's lines in the language of the file they're from (X-174). ada-parse's Diff lexer reads
//! a line alone; this reads a diff as `patch` does. A header names the file: `---` the old one,
//! `+++` the new one, which a rename may give another language (PGR-006). In a hunk, whose
//! header says how many of the old file's lines and the new one's it holds, a line is the old
//! file's (`-`), the new one's (`+`) or both's (context) by its first byte alone. Each side is
//! lexed in its file's language from a state of its own, both from a fresh one at each hunk (a
//! hunk starts mid-file, where a comment or a string may be open); a context line shows in the
//! new file's. A hunk header's section text is the new file's, from a fresh state too. A hunk
//! that counts more lines than it has (a patch edited by hand) ends at the next file's `---`,
//! `+++` and `@@`.
//!
//! A changed line keeps the diff's mark, its text the language's colors, and carries a tint
//! ([`Tint`]) for the painter to draw across it; a file named in a language with no lexer
//! (`README.md`) is tinted, its text plain. A file that isn't named reads as Diff's lexer reads
//! it, but a hunk's lines are still its lines: a removed `--- x` or an added `+++ x` is no file
//! header.
//!
//! A change block, a hunk's removed lines and the added ones after them, is read ahead at its
//! first line, and its lines' changed words found ([`words`], FEAT-023): they carry a stronger
//! tint, even where the file's language has no lexer. Only a block of at most [`BLOCK_LINES`]
//! lines and [`BLOCK_BYTES`] is, so that a streamed print's reader, which holds that much past
//! a piece ([`enough_ahead`]), reads it as the whole one does. Its lines are compared as the
//! diff has them, tabs and a `\r` too, and an added line's whitespace errors (blanks at its end,
//! spaces before a tab in its indentation) are on the deleted words' tint, as git colors them
//! (PGR-005).
//!
//! A diff without hunk headers (a ```` ```diff ```` fence, a pasted snippet) reads its changed
//! lines as a hunk's of no counts ([`Diff::loose`], PGR-005), until a header of git's shows.
//!
//! A commit's header in `git log -p` or `git show`, its `commit <hash>` line and the `Author:`,
//! `Date:` lines after it, is styled quietly ([`Preamble`]), so it doesn't draw the eye from the
//! code: the hash and its refs in color, the labels, punctuation, email and date dimmed, the
//! author and the message plain; `--pretty=raw`'s fields, `--left-right`'s mark and `-m`'s
//! `(from <hash>)` too (PGR-006). An annotated tag's header in `git show`, `tag <name>` and
//! `Tagger:`, `Date:` after it, is styled alike, its name a ref's, and its message plain to its
//! commit's header, a signature's `-----BEGIN PGP SIGNATURE-----` too (PGR-004). A
//! `format-patch` mail's, `From <hash>` and its `From:`, `Subject:` after it, is styled alike,
//! its message and diffstat plain to its first diff, and its `-- ` signature after its last hunk
//! plain to the next mail (PGR-006).

mod words;

use ada_parse::{DiffHeader, Lang, Rule, Run, Span, State};

use super::{LineRuns, Lines, OpenLine, ParseRules, Plain, Resolving, SyntaxTheme, Tint};

/// The most lines a change block's words are compared in, counting from its first line to the
/// line after it, `\ No newline` lines too.
const BLOCK_LINES: usize = 64;

/// The most bytes a change block's words are compared in, its lines' as [`BLOCK_LINES`] counts
/// them, a byte for each newline and none for a `\r` before it.
const BLOCK_BYTES: usize = 16 << 10;

/// Whether `lines` lines of `bytes` past a streamed diff's piece are enough to read a change
/// block that starts in it as the whole diff reads it (FEAT-023): a block read past either limit has
/// no changed words, and a line's bytes are at most twice what [`BLOCK_BYTES`] counts of it.
pub(crate) fn enough_ahead(lines: usize, bytes: usize) -> bool {
    lines > BLOCK_LINES || bytes > 2 * (BLOCK_BYTES + 1)
}

/// The lines after a diff's line: those left of the text being highlighted, then those of the
/// text after it that a streamed print's reader holds, each piece of it whole lines, the last
/// one's newline missing only at the end of the input.
#[derive(Clone, Copy)]
pub(super) struct Ahead<'a> {
    rest: Lines<'a>,
    more: &'a [&'a str],
}

impl<'a> Ahead<'a> {
    /// The next line with its `\r`, if it has one ([`Lines::next_raw`]).
    fn next_raw(&mut self) -> Option<&'a str> {
        loop {
            if let Some(line) = self.rest.next_raw() {
                return Some(line);
            }
            let (piece, more) = self.more.split_first()?;
            self.more = more;
            self.rest = Lines(match piece.strip_suffix('\n') {
                Some(lines) => Some(lines),
                None => Some(*piece).filter(|p| !p.is_empty()),
            });
        }
    }
}

impl<'a> Iterator for Ahead<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let line = self.next_raw()?;
        Some(line.strip_suffix('\r').unwrap_or(line))
    }
}

/// Which file a hunk's line is from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The new file's, `+`.
    New,
    /// The old file's, `-`.
    Old,
    /// Both's, a context line.
    Both,
}

const NONE: Rule = Rule(Rule::NONE);

/// A commit header's rules, resolved from scopes under `source.diff` whose last scope names a
/// kind every theme styles: dimmed, the hash's, a ref's.
struct Preamble {
    dim: Rule,
    hash: Rule,
    git_ref: Rule,
}

impl Preamble {
    fn new(theme: &SyntaxTheme) -> Self {
        let rule = |scope| theme.rule(&["source.diff", "meta.header.commit.diff", scope]);
        Preamble {
            dim: rule("comment.other.commit.diff"),
            hash: rule("constant.other.hash.diff"),
            git_ref: rule("entity.name.tag.ref.diff"),
        }
    }
}

/// Where a commit's header is being read, an annotated tag's or a mail's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Header {
    /// The `Author:`, `Date:` lines after `commit <hash>` (`author`, `tree`, `parent` in
    /// `--pretty=raw`), `Tagger:` after `tag <name>`, or a mail's `From:`, `Subject:` after
    /// `From <hash>`.
    Fields(Kind),
    /// The message, after the fields' blank line: plain, indented.
    Message,
    /// Text at the margin, plain until the next header: a tag's message and its signature, a
    /// mail's message and its diffstat, a mail's signature after its last hunk (`-- `).
    Plain,
}

/// Whose header's fields are being read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Commit,
    Tag,
    Mail,
}

/// The file a side of a diff's hunks is from, as its headers name it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum File {
    /// No header names one: a hunk's lines read as Diff's lexer reads them.
    Unnamed,
    /// One in no language ada lexes (`README.md`, `notes.txt`): its text plain, on the tint.
    Plain,
    /// One in a language ada lexes: an index into `langs`.
    Lang(usize),
}

/// A diff's reading carried from line to line.
pub(super) struct Diff {
    /// Diff's own rules.
    diff: ParseRules,
    /// The rules of the files' languages, each added when a header first names one.
    langs: Vec<ParseRules>,
    /// The new file's and the old one's ([`Side`]'s order): a rename may change the language.
    files: [File; 2],
    /// The paths the headers since the last hunk name, the old file's and the new one's, their
    /// languages not looked up yet: a file's headers name it at the first hunk, once.
    named: Option<(Option<String>, Option<String>)>,
    /// The C family's language of the last such file the diff named (`.c`, `.cpp`, `.m`, `.mm`):
    /// a `.h`'s, which can't be read ahead for (X-134 reads a header's text).
    c_family: Option<Lang>,
    /// The old file's lines and the new one's that the hunk has left; `None` outside a hunk.
    left: Option<(u32, u32)>,
    old: State,
    new: State,
    /// `+`'s and `-`'s rules, the mark's and the text's, as Diff's lexer gives them.
    marks: [(Rule, Rule); 2],
    spans: Vec<Span>,
    /// A line's runs as a lexer gives them, before they're laid into the block's.
    runs: Vec<Run>,
    /// The old side's runs of a context line, which only its state is wanted from.
    spare: Vec<Run>,
    /// The change block the line before was in: whether its added lines have begun.
    block: Option<bool>,
    /// The commit header being read, until a line that's no part of one.
    header: Option<Header>,
    preamble: Preamble,
    /// The block's lines' changed words, and the next line's place among them.
    words: words::Words,
    next: usize,
    /// An added line's whitespace errors, and its words runs where it has any.
    errors: Vec<(u32, u32)>,
    pieces: Vec<(u32, u32, u16)>,
    /// Whether no header of git's has shown yet (`diff`, `index`, `@@`, a commit's, a mail's):
    /// changed lines outside a hunk are read as a hunk's ([`Diff::loose`]).
    unhunked: bool,
    /// Whether a mail's header has shown (`From <hash>`): a `-- ` after its hunks starts its
    /// signature.
    mail: bool,
}

impl Diff {
    pub(super) fn new(theme: &SyntaxTheme, spans: Vec<Span>) -> Self {
        let mut d = Diff {
            diff: ParseRules::cached(Lang::Diff, theme.id),
            langs: Vec::new(),
            files: [File::Unnamed; 2],
            named: None,
            c_family: None,
            left: None,
            old: State::default(),
            new: State::default(),
            marks: [(NONE, NONE); 2],
            spans,
            runs: Vec::new(),
            spare: Vec::new(),
            block: None,
            header: None,
            preamble: Preamble::new(theme),
            words: words::Words::default(),
            next: 0,
            errors: Vec::new(),
            pieces: Vec::new(),
            unhunked: true,
            mail: false,
        };
        // the marks' rules as Diff's lexer has them, read from a changed line of each side
        for (i, line) in ["+x", "-x"].into_iter().enumerate() {
            d.diff_runs(line, theme);
            d.marks[i] = (rule_at(&d.runs, 0), rule_at(&d.runs, 1));
        }
        d
    }

    /// Hand the rules resolved to the blocks after this one, and the spans' vector back.
    pub(super) fn done(mut self) -> Vec<Span> {
        self.diff.keep();
        for rules in &mut self.langs {
            rules.keep();
        }
        self.spans
    }

    /// Highlight up to `max` of `text`'s lines into `out`, after the lines before them: where
    /// the lines left start, `None` once `text` is done (as [`ada_parse::lex_lines`]). When
    /// `expand`, the lines are expanded into `buf` first ([`super::lex_block`]). `more` is the
    /// text after `text` that a streamed print's reader holds ([`Ahead`]).
    pub(super) fn lines(
        &mut self,
        text: &str,
        max: usize,
        (buf, expand): (&mut String, bool),
        more: &[&str],
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) -> Option<usize> {
        if !expand {
            return self.split(text, None, max, more, theme, out);
        }
        if max == 0 {
            return Some(0);
        }
        let next = super::expand_block(text, max, buf);
        let done = self.split(buf, Some(text), usize::MAX, more, theme, out);
        debug_assert!(done.is_none(), "every expanded line highlighted");
        next
    }

    /// [`Diff::lines`] on lines as they are, or expanded from `raw`'s, which a block is read
    /// ahead in.
    fn split(
        &mut self,
        text: &str,
        raw: Option<&str>,
        max: usize,
        more: &[&str],
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) -> Option<usize> {
        let mut raw = raw.map(|r| Lines(Some(r)));
        let mut at = 0;
        for _ in 0..max {
            let rest = &text[at..];
            let (line, next) = match memchr::memchr(b'\n', rest.as_bytes()) {
                Some(i) => (&rest[..i], Some(at + i + 1)),
                None => (rest, None),
            };
            // the line as the diff has it, its tabs and `\r`, which its words are told by
            let (own, rest) = match &mut raw {
                Some(raw) => (raw.next_raw().unwrap_or(line), *raw),
                None => (line, Lines(next.map(|n| &text[n..]))),
            };
            let line = line.strip_suffix('\r').unwrap_or(line);
            let ahead = Ahead { rest, more };
            self.line((line, own), ahead, theme, out);
            at = next?;
        }
        Some(at)
    }

    /// Highlight `line` into `out`, from the lines before it alone, but for a change block's
    /// first line, which reads the block's lines after it in `ahead`: a streamed diff reads as
    /// the whole one does. `own` is the line as the diff has it, before tabs are expanded and
    /// its `\r` dropped.
    fn line(
        &mut self,
        (line, own): (&str, &str),
        ahead: Ahead<'_>,
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) {
        self.loose(line, ahead, theme);
        let left = self.left;
        if let Some(side) = self.body(line, ahead) {
            self.block(own, side, left, ahead);
            return self.body_line((line, own), side, theme, out);
        }
        // `\ No newline at end of file` is between a block's lines
        if !line.starts_with('\\') {
            self.block = None;
        }
        let mut next = ahead;
        if self.header_line(line, next.next(), out) {
            self.unhunked = false;
            return;
        }
        if let Some(hunk) = ada_parse::diff_hunk(line) {
            self.unhunked = false;
            self.resolve(theme);
            self.left = (hunk.old > 0 || hunk.new > 0).then_some((hunk.old, hunk.new));
            (self.old, self.new) = (State::default(), State::default());
            let (s, e) = hunk.section;
            return self.hunk_line(line, (s as u32, e as u32), theme, out);
        }
        if let Some(header) = ada_parse::diff_header(line) {
            if matches!(header, DiffHeader::Diff(_) | DiffHeader::Index(_)) {
                self.unhunked = false;
            }
            self.name(header);
        }
        self.diff_runs(line, theme);
        let mut open = out.open_line();
        for r in &self.runs {
            open.push(r.end, r.rule);
        }
        open.end();
    }

    /// A commit header's `line`, an annotated tag's or a mail's, styled quietly, or a mail's
    /// signature, plain: false when it's no part of one. `next` is the line after it.
    fn header_line(&mut self, line: &str, next: Option<&str>, out: &mut LineRuns) -> bool {
        if let Some((mark, hash)) = commit_line(line) {
            self.header = Some(Header::Fields(Kind::Commit));
            let p = &self.preamble;
            let mut open = Runs::new(out.open_line());
            open.to(6, p.dim);
            // `--left-right`'s `<` or `>`, `--boundary`'s `-`, `--cherry-mark`'s `=` or `+`
            let at = 7 + mark;
            open.to(at as u32, NONE);
            open.to((at + hash.len()) as u32, p.hash);
            refs(&mut open, p, line, at + hash.len());
            open.end();
            return true;
        }
        // `git show` of an annotated tag: `tag <name>`, then `Tagger:`
        if let Some(name) = line.strip_prefix("tag ")
            && !name.contains(' ')
            && next.is_some_and(|n| n.starts_with("Tagger: "))
        {
            self.header = Some(Header::Fields(Kind::Tag));
            let p = &self.preamble;
            let mut open = Runs::new(out.open_line());
            open.to(3, p.dim);
            open.to(4, NONE);
            open.to(line.len() as u32, p.git_ref);
            open.end();
            return true;
        }
        // a `format-patch` mail's mbox line, `From <hash> <date>`
        if let Some(hash) = mbox_from(line) {
            self.header = Some(Header::Fields(Kind::Mail));
            self.mail = true;
            let p = &self.preamble;
            let mut open = Runs::new(out.open_line());
            open.to(4, p.dim);
            open.to(5, NONE);
            open.to(5 + hash.len() as u32, p.hash);
            open.to(line.len() as u32, p.dim);
            open.end();
            return true;
        }
        // a mail's signature after its last hunk: `-- `, then git's version
        if self.mail && self.left.is_none() && line == "-- " {
            self.header = Some(Header::Plain);
            out.open_line().end();
            return true;
        }
        match (self.header, line) {
            (None, _) => return false,
            // the fields' blank line, then the message
            (Some(Header::Fields(Kind::Commit)), "") => self.header = Some(Header::Message),
            (Some(Header::Fields(_)), "") => self.header = Some(Header::Plain),
            (Some(Header::Message), l) if l.is_empty() || l.starts_with("    ") => {}
            (Some(Header::Plain), l) if !l.starts_with("diff ") && !l.starts_with("Index: ") => {}
            (Some(Header::Fields(kind)), l) if self.field(kind, l, out) => return true,
            _ => {
                self.header = None;
                return false;
            }
        }
        out.open_line().end();
        true
    }

    /// A header's `Label: value` line, its label dimmed; a raw commit's `label value`
    /// (`--pretty=raw`), or a line that goes on the one before it (a mail's long `Subject:`, a raw
    /// commit's signature): false when it's none of them.
    fn field(&self, kind: Kind, line: &str, out: &mut LineRuns) -> bool {
        let p = &self.preamble;
        let len = line.len() as u32;
        if line.starts_with([' ', '\t']) {
            let mut open = Runs::new(out.open_line());
            open.to(len, if kind == Kind::Mail { NONE } else { p.dim });
            open.end();
            return true;
        }
        if kind == Kind::Commit
            && let Some((label, value)) = line.split_once(' ')
            && matches!(
                label,
                "tree" | "parent" | "author" | "committer" | "encoding" | "gpgsig" | "mergetag"
            )
        {
            let at = label.len() as u32 + 1;
            let mut open = Runs::new(out.open_line());
            open.to(at, p.dim);
            match label {
                "tree" | "parent" => open.to(len, p.hash),
                // `Name <email> 1700000000 +0100`: the email and the time dimmed
                "author" | "committer" => {
                    if let Some(lt) = value.rfind('<') {
                        open.to(at + lt as u32, NONE);
                    }
                    open.to(len, p.dim);
                }
                _ => open.to(len, p.dim),
            }
            open.end();
            return true;
        }
        let Some((label, value)) = line.split_once(':').filter(|(l, _)| {
            !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
        }) else {
            return false;
        };
        let at = label.len() as u32 + 1;
        let mut open = Runs::new(out.open_line());
        open.to(at, p.dim);
        match label {
            // `Name <email>`: the email dimmed
            "Author" | "Commit" | "Tagger" | "From" => {
                if let Some(lt) = value.rfind('<') {
                    open.to(at + lt as u32, NONE);
                }
                open.to(len, p.dim);
            }
            "Merge" => {
                let mut end = at;
                for word in value.split(' ') {
                    open.to(end, NONE);
                    end += word.len() as u32;
                    open.to(end, p.hash);
                    end += 1;
                }
            }
            // `Date:`, and `AuthorDate:`, `CommitDate:` in `--format=fuller`
            _ if label.ends_with("Date") => open.to(len, p.dim),
            // a mail's `Subject:` and its message are its text
            "Subject" => open.to(len, NONE),
            _ if kind == Kind::Mail => open.to(len, p.dim),
            _ => open.to(len, NONE),
        }
        open.end();
        true
    }

    /// A diff with no hunk headers (a ```` ```diff ```` fence, a pasted snippet), until a header of
    /// git's shows (PGR-005): a `-` or `+` line outside a hunk starts one of no counts
    /// ([`LOOSE`]), which a line neither changed nor context ends, or a file's `---` and `+++`
    /// headers.
    fn loose(&mut self, line: &str, ahead: Ahead<'_>, theme: &SyntaxTheme) {
        if !self.unhunked {
            return;
        }
        match self.left {
            Some(LOOSE) => {
                if file_follows(line, ahead, true) {
                    self.left = None;
                }
            }
            None if matches!(line.as_bytes().first(), Some(b'-' | b'+'))
                && !line.starts_with("---")
                && !line.starts_with("+++") =>
            {
                self.resolve(theme);
                self.left = Some(LOOSE);
                (self.old, self.new) = (State::default(), State::default());
            }
            _ => {}
        }
    }

    /// Which side a hunk's `line` is from, its count taken: `None` outside a hunk, and for a
    /// line that ends it early (a hunk's lines miscounted, as a patch edited by hand has them:
    /// a line neither side has room for, or a file's `---`, `+++` and `@@`, PGR-006). `ahead`
    /// are the lines after it.
    fn body(&mut self, line: &str, ahead: Ahead<'_>) -> Option<Side> {
        let loose = self.left == Some(LOOSE);
        let (old, new) = self.left.as_mut()?;
        let side = match line.as_bytes().first() {
            Some(b'+') if *new > 0 => {
                *new -= 1;
                Side::New
            }
            Some(b'-') if *old > 0 && !(!loose && file_follows(line, ahead, false)) => {
                *old -= 1;
                Side::Old
            }
            // an empty context line, as some tools write one
            Some(b' ') | None if *old > 0 && *new > 0 => {
                *old -= 1;
                *new -= 1;
                Side::Both
            }
            // `\ No newline at end of file`, after either side's last line: Diff's, the hunk
            // going on
            Some(b'\\') => return None,
            _ => {
                self.left = None;
                return None;
            }
        };
        if loose {
            // a loose hunk's lines aren't counted
            self.left = Some(LOOSE);
        } else if (*old, *new) == (0, 0) {
            self.left = None;
        }
        Some(side)
    }

    /// Where a hunk's line, `own` as the diff has it, of `side`, the hunk's counts `left` before
    /// it, is in a change block: at a block's first line, the block is read in `ahead` and its
    /// words found.
    fn block(&mut self, own: &str, side: Side, left: Option<(u32, u32)>, ahead: Ahead<'_>) {
        match (side, self.block) {
            (Side::Both, _) => self.block = None,
            (Side::Old, Some(false)) | (Side::New, Some(_)) => {
                self.next += 1;
                self.block = Some(side == Side::New);
            }
            (Side::Old | Side::New, _) => {
                self.block = Some(side == Side::New);
                self.next = 0;
                // a block whose first line is added, or whose hunk adds no more lines, is one
                // side's alone: no pairs
                let read = match (side, left) {
                    (Side::Old, Some((_, new))) if new > 0 => read_block(own, left, ahead),
                    _ => None,
                };
                match read {
                    Some((lines, old)) => self.words.block(&lines, old),
                    None => self.words.block(&[], 0),
                }
            }
        }
    }

    /// A hunk's line: its mark and its text in the file's language, or as Diff's lexer has a
    /// changed line (context plain) when there is none. A changed line's words runs come first:
    /// its changed words, an added line's whitespace errors on a deleted word's tint, as git
    /// colors them red, and its end's mark ([`words::END`]). `own` is the line as the diff has
    /// it.
    fn body_line(
        &mut self,
        (line, own): (&str, &str),
        side: Side,
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) {
        let mut open = out.open_line();
        let len = line.len() as u32;
        if side != Side::Both {
            let tint = match side {
                Side::New => Tint::INSERTED,
                _ => Tint::DELETED,
            };
            if self.files[side as usize] != File::Unnamed {
                open.push(0, Rule(tint));
            }
            let words = match side {
                Side::New => Tint::WORDS_INSERTED,
                _ => Tint::WORDS_DELETED,
            };
            let changed = self.words.line(self.next);
            self.errors.clear();
            if side == Side::New {
                words::whitespace_errors(own, &mut self.errors);
            }
            if self.errors.is_empty() {
                for &(a, b) in changed {
                    open.push(a, Rule(Tint::SAME));
                    open.push(b, Rule(words));
                }
            } else {
                // the changed words but where an error is, and the errors, in order
                let pieces = &mut self.pieces;
                pieces.clear();
                for &(mut a, b) in changed {
                    for &(x, y) in &self.errors {
                        if x < b && y > a {
                            if x > a {
                                pieces.push((a, x, words));
                            }
                            a = y;
                        }
                    }
                    if b > a {
                        pieces.push((a, b, words));
                    }
                }
                let errors = self
                    .errors
                    .iter()
                    .map(|&(a, b)| (a, b, Tint::WORDS_DELETED));
                pieces.extend(errors);
                pieces.sort_unstable_by_key(|m| m.0);
                for &(a, b, rule) in pieces.iter() {
                    open.push(a, Rule(Tint::SAME));
                    open.push(b, Rule(rule));
                }
            }
        }
        // the language the line shows in: a context line's the new file's
        let shown = self.files[match side {
            Side::Old => Side::Old,
            _ => Side::New,
        } as usize];
        if shown == File::Unnamed {
            if side != Side::Both {
                let (mark, text) = self.marks[side as usize];
                open.push(1, mark);
                if len > 1 {
                    open.push(len, text);
                }
            }
            return open.end();
        }
        // the mark is one byte, ASCII, or the line is empty
        let text = line.get(1..).unwrap_or_default();
        self.runs.clear();
        match side {
            Side::Old => self.lex(Side::Old, text, theme, true),
            Side::New => self.lex(Side::New, text, theme, true),
            Side::Both => {
                let before = self.new;
                self.lex(Side::New, text, theme, true);
                // the old side, from where it is: the new side's state when they're the same
                match self.files {
                    [File::Lang(i), File::Lang(j)] if i == j && self.old == before => {
                        self.old = self.new;
                    }
                    _ => self.lex(Side::Old, text, theme, false),
                }
            }
        }
        if side == Side::Both {
            if !self.runs.is_empty() {
                open.push(1, NONE);
            }
        } else {
            open.push(1, self.marks[side as usize].0);
        }
        for r in &self.runs {
            open.push(r.end + 1, r.rule);
        }
        open.end();
    }

    /// Lex `text`, a hunk line's after its mark, on `side`'s state in its file's language:
    /// into `runs` when `shown`, else for the state alone (a context line's old side). Nothing
    /// for a file in no language ada lexes.
    fn lex(&mut self, side: Side, text: &str, theme: &SyntaxTheme, shown: bool) {
        let File::Lang(i) = self.files[side as usize] else {
            return;
        };
        let rules = &mut self.langs[i];
        let lang = rules.lang;
        let state = match side {
            Side::Old => &mut self.old,
            _ => &mut self.new,
        };
        if shown {
            let resolving = &mut Resolving { rules, theme };
            ada_parse::lex_runs(
                lang,
                text,
                state,
                &mut self.spans,
                resolving,
                &mut self.runs,
            );
        } else {
            self.spare.clear();
            ada_parse::lex_runs(
                lang,
                text,
                state,
                &mut self.spans,
                &mut Plain,
                &mut self.spare,
            );
        }
    }

    /// A hunk header, as Diff's lexer has it but its section text, bytes `section`, in the file's
    /// language from a fresh state.
    fn hunk_line(
        &mut self,
        line: &str,
        (s, e): (u32, u32),
        theme: &SyntaxTheme,
        out: &mut LineRuns,
    ) {
        self.diff_runs(line, theme);
        let mut open = out.open_line();
        let section = match self.files[Side::New as usize] {
            File::Lang(i) if s < e => {
                let rules = &mut self.langs[i];
                let (lang, text) = (rules.lang, &line[s as usize..e as usize]);
                self.spare.clear();
                let resolving = &mut Resolving { rules, theme };
                let state = &mut State::default();
                ada_parse::lex_runs(
                    lang,
                    text,
                    state,
                    &mut self.spans,
                    resolving,
                    &mut self.spare,
                );
                !self.spare.is_empty()
            }
            _ => false,
        };
        if !section {
            for r in &self.runs {
                open.push(r.end, r.rule);
            }
            return open.end();
        }
        clipped(&mut open, &self.runs, 0, s);
        for r in &self.spare {
            open.push(r.end + s, r.rule);
        }
        clipped(&mut open, &self.runs, e, line.len() as u32);
        open.end();
    }

    /// The file a header names, its language looked up at the first hunk: a new file's section
    /// (`diff …`) forgets the one before, a combined diff's names none. `---` names the old file
    /// and `+++` the new one, which a rename to another extension tells apart (PGR-006).
    fn name(&mut self, header: DiffHeader<'_>) {
        let (old, new) = match header {
            DiffHeader::Diff(path) => {
                self.files = [File::Unnamed; 2];
                self.named = path.map(|p| (Some(p.to_owned()), Some(p.to_owned())));
                return;
            }
            // a file added or deleted: the other side's name stands
            DiffHeader::From(None) | DiffHeader::To(None) => return,
            DiffHeader::From(Some(p)) => (Some(p), None),
            DiffHeader::To(Some(p)) => (None, Some(p)),
            DiffHeader::Index(p) => (Some(p), Some(p)),
        };
        let named = self.named.get_or_insert_default();
        for (side, path) in [(&mut named.0, old), (&mut named.1, new)] {
            if let Some(p) = path {
                let name = side.get_or_insert_default();
                name.clear();
                name.push_str(p);
            }
        }
    }

    /// The files the headers since the last hunk named, their languages looked up: a side not
    /// named takes the other's.
    fn resolve(&mut self, theme: &SyntaxTheme) {
        let Some((old, new)) = self.named.take() else {
            return;
        };
        let (old, new) = (
            old.as_deref().or(new.as_deref()),
            new.as_deref().or(old.as_deref()),
        );
        let old_file = old.map_or(File::Unnamed, |p| self.language(p, theme));
        let new_file = match new == old {
            true => old_file,
            false => new.map_or(File::Unnamed, |p| self.language(p, theme)),
        };
        self.files = [new_file, old_file];
    }

    /// The language of the file at `path`: an index into `langs`, the language's rules added
    /// when it's new; [`File::Plain`] for one in no language ada lexes.
    fn language(&mut self, path: &str, theme: &SyntaxTheme) -> File {
        let Some(lang) = language(path, self.c_family) else {
            return File::Plain;
        };
        if matches!(lang, Lang::C | Lang::Cpp | Lang::ObjC | Lang::ObjCpp) {
            self.c_family = Some(lang);
        }
        File::Lang(match self.langs.iter().position(|r| r.lang == lang) {
            Some(i) => i,
            None => {
                self.langs.push(ParseRules::cached(lang, theme.id));
                self.langs.len() - 1
            }
        })
    }

    /// `line`'s runs as Diff's lexer has them, into `runs`.
    fn diff_runs(&mut self, line: &str, theme: &SyntaxTheme) {
        self.runs.clear();
        let resolving = &mut Resolving {
            rules: &mut self.diff,
            theme,
        };
        let state = &mut State::default();
        ada_parse::lex_runs(
            Lang::Diff,
            line,
            state,
            &mut self.spans,
            resolving,
            &mut self.runs,
        );
    }
}

/// Read the change block `first` starts, the hunk's counts `left` before it, as [`Diff::body`]
/// will read its lines: its lines as the diff has them (tabs, a `\r` at the end), and how many
/// are removed lines; `None` when the block is past [`BLOCK_LINES`] or [`BLOCK_BYTES`].
fn read_block<'a>(
    first: &'a str,
    left: Option<(u32, u32)>,
    ahead: Ahead<'a>,
) -> Option<(Vec<&'a str>, usize)> {
    let (mut old, mut new) = left?;
    let (mut lines, mut bytes, mut removed) = (0, 0, 0);
    let mut block = Vec::new();
    // `after` the lines after `line`
    let (mut line, mut after) = (first, ahead);
    loop {
        lines += 1;
        bytes += line.strip_suffix('\r').unwrap_or(line).len() + 1;
        if lines > BLOCK_LINES || bytes > BLOCK_BYTES {
            return None;
        }
        // a hunk's lines end at a file's headers, as [`Diff::body`] and [`Diff::loose`] end them
        if file_follows(line, after, left == Some(LOOSE)) {
            break;
        }
        let pushed = match line.as_bytes().first() {
            Some(b'+') if new > 0 => {
                new -= 1;
                true
            }
            Some(b'-') if old > 0 && removed == block.len() => {
                old -= 1;
                removed += 1;
                true
            }
            Some(b'\\') => false,
            _ => break,
        };
        if pushed {
            block.push(line);
            if (old, new) == (0, 0) {
                break;
            }
        }
        match after.next_raw() {
            Some(next) => line = next,
            None => break,
        }
    }
    Some((block, removed))
}

/// The longest `+++` line [`file_follows`] takes for a header: past it, a streamed print may hold
/// only its start, and not the line after it (PGR-008's bounded read-ahead).
const HEADER_MAX: usize = 4 << 10;

/// Whether `line` is a file's `---` header, `after` the lines after it: a `+++` follows it, and,
/// unless the hunk is `loose`, an `@@` that. In a hunk that counts more lines than it has, a
/// patch edited by hand, the next file's headers end it (PGR-006).
fn file_follows(line: &str, mut after: Ahead<'_>, loose: bool) -> bool {
    line.starts_with("--- ")
        && after
            .next()
            .is_some_and(|n| n.starts_with("+++ ") && n.len() <= HEADER_MAX)
        && (loose || after.next().is_some_and(|n| n.starts_with("@@ ")))
}

/// A loose hunk's counts ([`Diff::loose`]): more lines than any diff has.
const LOOSE: (u32, u32) = (u32::MAX, u32::MAX);

/// The hash of an mbox's `From <hash> <date>`, a `format-patch` mail's first line.
fn mbox_from(line: &str) -> Option<&str> {
    line.strip_prefix("From ")
        .and_then(|r| r.split(' ').next())
        .filter(|h| matches!(h.len(), 40 | 64) && is_hash(h))
}

/// A `commit <hash>` line's hash, and the length of the mark before it with its space: `<` or
/// `>` (`--left-right`), `-` (`--boundary`), `=` or `+` (`--cherry-mark`), `^`; 0 for none.
fn commit_line(line: &str) -> Option<(usize, &str)> {
    let rest = line.strip_prefix("commit ")?;
    let (mark, rest) = match rest.as_bytes() {
        [b'<' | b'>' | b'-' | b'=' | b'+' | b'^', b' ', ..] => (2, &rest[2..]),
        _ => (0, rest),
    };
    let hash = rest.split(' ').next()?;
    (hash.len() >= 7 && is_hash(hash)).then_some((mark, hash))
}

/// Whether `word` is all hex digits, as a hash is.
fn is_hash(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The language a diff's file `path` is in, among those ada-parse lexes, by its name as ada-parse
/// names a file. A `.h` (and its CMake template) is `c_family`'s, the C family's last file in the
/// diff, a header being mostly of its neighbors' language (X-134); with none, what X-134 reads
/// of no text. Not by its own text, as a file's: that lies past what a streamed print has read,
/// and a streamed diff must read as the whole one does.
fn language(path: &str, c_family: Option<Lang>) -> Option<Lang> {
    let name = path.rsplit('/').next()?;
    if let Some(alone) = crate::filetype::header_by_text(name, "") {
        let cpp = matches!(c_family, Some(Lang::Cpp | Lang::ObjCpp));
        return Some(match alone {
            Lang::CMakeCHeader | Lang::CMakeCppHeader if cpp => Lang::CMakeCppHeader,
            Lang::CMakeCHeader | Lang::CMakeCppHeader => Lang::CMakeCHeader,
            alone => c_family.unwrap_or(alone),
        });
    }
    // ada-parse's naming alone: only a language it lexes is wanted, and bat's set's lists cost
    // a search a file (X-174: three fifths of Diff's corpus's time)
    Lang::from_file_name(name)
        .or_else(|| Lang::strip_ignored_suffix(name).and_then(Lang::from_file_name))
}

/// The rule `runs` give byte `at`, plain past them.
/// A line's runs pushed in order, an empty one skipped.
struct Runs<'a> {
    open: OpenLine<'a>,
    at: u32,
}

impl<'a> Runs<'a> {
    fn new(open: OpenLine<'a>) -> Self {
        Runs { open, at: 0 }
    }

    /// The bytes up to `end` take `rule`.
    fn to(&mut self, end: u32, rule: Rule) {
        if end > self.at {
            self.open.push(end, rule);
            self.at = end;
        }
    }

    fn end(self) {
        self.open.end();
    }
}

/// A commit line's words after its hash, bytes `at..`: `--parents`' hashes, `-m`'s
/// ` (from <hash>)`, and refs ` (HEAD -> main, tag: v1.0)`, the hashes and refs in their color
/// and the rest dimmed (PGR-006: `from` is no ref).
fn refs(open: &mut Runs<'_>, p: &Preamble, line: &str, at: usize) {
    let mut end = at as u32;
    // in parentheses: whether they're `(from …)`'s
    let mut group = None;
    for word in line[at..].split(' ') {
        let start = end;
        end += word.len() as u32;
        if !word.is_empty() {
            let name = word.trim_start_matches('(');
            if name.len() < word.len() {
                group = Some(name == "from");
            }
            let lead = start + (word.len() - name.len()) as u32;
            let closes = name.ends_with(')');
            let name = name.trim_end_matches([',', ')']);
            let rule = match group {
                None if is_hash(name) => p.hash,
                Some(true) if name != "from" => p.hash,
                Some(true) => p.dim,
                _ if matches!(name, "->" | "tag:") => p.dim,
                _ => p.git_ref,
            };
            open.to(start, NONE);
            open.to(lead, p.dim);
            open.to(lead + name.len() as u32, rule);
            open.to(end, p.dim);
            if closes {
                group = None;
            }
        }
        end += 1;
    }
}

fn rule_at(runs: &[Run], at: u32) -> Rule {
    runs.iter().find(|r| r.end > at).map_or(NONE, |r| r.rule)
}

/// `runs`' rules over bytes `from..to`, onto a line whose runs so far end at `from`; plain where
/// they end before `to`.
fn clipped(open: &mut OpenLine<'_>, runs: &[Run], from: u32, to: u32) {
    let mut start = 0;
    for r in runs {
        if start >= to {
            return;
        }
        if r.end > from {
            open.push(r.end.min(to), r.rule);
        }
        start = r.end;
    }
    if start < to {
        open.push(to, NONE);
    }
}
