//! TeX math as Unicode text (OBS-003 step 2): `$\alpha \le \frac{n}{2}$` reads `α ≤ n/2`. A
//! reader of the math Obsidian notes hold, table-driven and without a dependency; a typesetter
//! for the terminal, not TeX.
//!
//! - **Symbols:** Greek, operators, relations, arrows and the rest of the common commands, from
//!   `SYMBOLS`; function names (`\sin`, `\lim`) as words.
//! - **Alphabets:** `\mathbb`, `\mathcal`, `\mathfrak` and `\mathbf` in Unicode's math letters
//!   (`\mathbb{R}` is ℝ); `\text` and `\mathrm` as written.
//! - **Scripts:** in Unicode's sub- and superscript letters where it has every one (`x_1` is x₁,
//!   `n^2` n²), else written out, `e^(iπ)`.
//! - **Fractions and roots** inline: `\frac{a+b}{2}` is (a+b)/2, `\sqrt{x}` √x.
//! - **Spacing** as TeX's atom classes give it, whatever the source's: a space around a relation
//!   or a binary operator (but a sign), after a comma and between a function's name and its
//!   argument, so `x^2+y^2=z^2` reads `x² + y² = z²`.
//! - **Rows** of display math (`\\`), aligned on their `&`s, and matrices and cases drawn with
//!   tall brackets, a row of text each.
//!
//! What it doesn't know (a macro of the note's own, `\binom` inline, a stray `&`) makes the
//! whole formula `None`, and it stays TeX as written: never half typeset.

use std::borrow::Cow;
use std::ops::Range;

use unicode_width::UnicodeWidthStr;

/// A space that holds: the wrap neither breaks a line at it nor folds a run of them into one, so
/// it pads aligned rows and wide spaces. Shown as a space.
const NBSP: char = '\u{a0}';

/// The deepest nesting of groups and arguments read; past it, `None`.
const MAX_DEPTH: u32 = 64;

/// A row of typeset display math: its text, and the bytes of the TeX its line came from. A tall
/// line (a matrix) is several rows of the same bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub tex: Range<usize>,
}

/// Inline math as one line of Unicode, or `None` if any of it is past what this reads.
pub fn inline(tex: &str) -> Option<String> {
    if prose(tex) {
        return None;
    }
    let mut p = Parser::new(github(tex), false);
    let (out, end) = p.seq(Stop::End, Ctx::Line)?;
    if end != Ended::End {
        return None;
    }
    let pic = out.finish();
    match &pic.rows[..] {
        [row] if !row.is_empty() => Some(row.clone()),
        _ => None,
    }
}

/// Display math as rows of Unicode, or `None` if any of it is past what this reads. Its lines
/// (`\\`) are rows aligned on their `&`s; one that is a single aligned environment
/// (`\begin{aligned}…\end{aligned}`) is read as its lines.
pub fn display(tex: &str) -> Option<Vec<Row>> {
    if prose(tex) {
        return None;
    }
    let env = whole_env(tex).and_then(|(name, body, at)| Some((body, at, ColAlign::of_env(name)?)));
    let (body, base, align) = env.unwrap_or((tex, 0, ColAlign::Lines));
    let mut p = Parser::new(body, true);
    let mut table = p.table(Stop::End, align)?;
    let pic = table.layout(align);
    let mut out = Vec::new();
    // each line's rows, from the top
    let mut row = 0;
    for (i, line) in table.rows.iter().enumerate() {
        let tex = trimmed(body, line.tex.clone());
        let tex = base + tex.start..base + tex.end;
        for _ in 0..table.heights[i] {
            if let Some(text) = pic.rows.get(row) {
                out.push(Row {
                    text: trim_end(text),
                    tex: tex.clone(),
                });
            }
            row += 1;
        }
    }
    // a line of nothing (a `\\` at the end) shows none
    out.retain(|r| !r.text.trim_matches([' ', NBSP]).is_empty());
    // the rows' common indent, from alignment, goes
    let indent = (out.iter())
        .map(|r| {
            r.text
                .chars()
                .take_while(|&c| c == NBSP || c == ' ')
                .count()
        })
        .min()
        .unwrap_or(0);
    if indent > 0 {
        for r in &mut out {
            r.text = r.text.chars().skip(indent).collect();
        }
    }
    (!out.is_empty()).then_some(out)
}

fn trim_end(s: &str) -> String {
    s.trim_end_matches([' ', NBSP]).to_owned()
}

/// `range` of `s` without the whitespace at its ends.
fn trimmed(s: &str, range: Range<usize>) -> Range<usize> {
    let Some(t) = s.get(range.clone()) else {
        return range;
    };
    let start = range.start + (t.len() - t.trim_start().len());
    let end = range.end - (t.len() - t.trim_end().len());
    start..end.max(start)
}

/// Whether `tex` reads as prose or code that two `$`s happen to hold, not TeX: sentence punctuation
/// of CJK text, or two words of letters side by side, outside an argument that holds text
/// (`\text{two words}` is TeX). Typeset, its spaces would go, as MathJax drops them (`$name its me$` as
/// `nameitsme`); as written, it reads. A guess: of the survey's 74,860 formulas it takes 303,
/// shell and prose but for a few that write CJK punctuation into TeX.
fn prose(tex: &str) -> bool {
    let cjk = |c: char| {
        matches!(
            c,
            '，' | '。'
                | '、'
                | '！'
                | '？'
                | '：'
                | '；'
                | '「'
                | '」'
                | '『'
                | '』'
                | '（'
                | '）'
                | '【'
                | '】'
                | '《'
                | '》'
                | '～'
        )
    };
    // two runs of two letters or more with only spaces between them, `name its`; a command's
    // name ends a run, and an argument that holds text is passed over
    let (mut last, mut run, mut gap) = (0, 0, false);
    let mut chars = tex.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '\\' {
            (last, run, gap) = (0, 0, false);
            let mut end = at + 1;
            while let Some((i, c)) = chars.next_if(|(_, c)| c.is_ascii_alphabetic()) {
                end = i + c.len_utf8();
            }
            let name = &tex[at + 1..end];
            let text = name.starts_with("text")
                || matches!(
                    name,
                    "mbox" | "hbox" | "emph" | "label" | "tag" | "color" | "begin" | "end"
                );
            if text && chars.next_if(|&(_, c)| c == '{').is_some() {
                let mut depth = 1;
                for (_, c) in chars.by_ref() {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                }
            }
        } else if c.is_ascii_alphabetic() {
            run += 1;
            if gap && last >= 2 && run >= 2 {
                return true;
            }
        } else if matches!(c, ' ' | '\t') && (run > 0 || gap) {
            if run > 0 {
                (last, run) = (run, 0);
            }
            gap = true;
        } else if cjk(c) {
            return true;
        } else {
            (last, run, gap) = (0, 0, false);
        }
    }
    false
}

/// The math of GitHub's `` $`…`$ ``, its backticks off.
fn github(tex: &str) -> &str {
    let inner = tex.strip_prefix('`').and_then(|t| t.strip_suffix('`'));
    inner.filter(|t| !t.is_empty()).unwrap_or(tex)
}

/// `tex` as one environment of rows and nothing else: its name, body and the body's offset.
fn whole_env(tex: &str) -> Option<(&str, &str, usize)> {
    let t = tex.trim_start();
    let lead = tex.len() - t.len();
    let rest = t.strip_prefix("\\begin{")?;
    let close = rest.find('}')?;
    let name = &rest[..close];
    let body_at = lead + "\\begin{".len() + close + 1;
    let end = format!("\\end{{{name}}}");
    let body_end = tex.trim_end().strip_suffix(&end)?.len();
    let body = tex.get(body_at..body_end)?;
    // the body holds no other end of it: `\begin{a}x\end{a} y \begin{a}z\end{a}`
    if body.contains(&end) {
        return None;
    }
    Some((name, body, body_at))
}

/// TeX's classes of atoms, as far as spacing tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Ord,
    /// A big operator or a function's name: `∑`, `sin`.
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
}

/// What a sequence was read up to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// Its stop: the end of the text, a `}`, a `]`.
    End,
    /// A `&`: the next cell of the row.
    Cell,
    /// A `\\`: the next row.
    Row,
    /// `\end{…}`, its name not yet read.
    Env,
    /// `\right`, its delimiter not yet read.
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// The end of the text.
    End,
    /// A `}`, which is taken.
    Brace,
    /// A `]`, which is taken (`\sqrt[3]`).
    Bracket,
    /// `\end{…}`.
    Env,
    /// `\right`.
    Right,
}

/// Where a sequence is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctx {
    /// On a line of its own: a group, an argument. Rows and cells end nothing here.
    Line,
    /// A cell of rows: `&` and `\\` end it.
    Cell,
    /// A script, set tight: no spacing.
    Script,
}

/// A typeset piece: rows of text, its baseline the row it sits on. Rows are padded with
/// [`NBSP`]s to the same width.
#[derive(Clone, Debug, Default)]
struct Pic {
    rows: Vec<String>,
    base: usize,
    width: usize,
}

/// [`Pic::width`] of a line not yet measured: most math is one line, which nothing measures.
const UNMEASURED: usize = usize::MAX;

impl Pic {
    fn line(s: String) -> Self {
        Self {
            rows: vec![s],
            base: 0,
            width: UNMEASURED,
        }
    }

    /// Its columns.
    fn cols(&self) -> usize {
        match self.width {
            UNMEASURED => self.rows.first().map_or(0, |r| r.width()),
            w => w,
        }
    }

    fn is_line(&self) -> bool {
        self.rows.len() == 1
    }

    /// `self` with `other` to its right, their baselines on a row.
    fn beside(mut self, other: Pic) -> Pic {
        if self.rows.is_empty() {
            return other;
        }
        self.width = self.cols();
        let other_cols = other.cols();
        let above = self.base.max(other.base);
        let below = (self.rows.len() - self.base).max(other.rows.len() - other.base);
        self.grow(above, below);
        let at = above - other.base;
        for (i, row) in self.rows.iter_mut().enumerate() {
            match i.checked_sub(at).and_then(|k| other.rows.get(k)) {
                Some(r) => row.push_str(r),
                None => pad(row, other_cols),
            }
        }
        self.width += other_cols;
        self
    }

    /// Blank rows added above and below to give `above` rows over the baseline and `below` from
    /// it down.
    fn grow(&mut self, above: usize, below: usize) {
        let blank: String = std::iter::repeat_n(NBSP, self.cols()).collect();
        let top = above - self.base;
        let bottom = below - (self.rows.len() - self.base);
        let mut rows = Vec::with_capacity(above + below);
        rows.extend(std::iter::repeat_n(blank.clone(), top));
        rows.append(&mut self.rows);
        rows.extend(std::iter::repeat_n(blank, bottom));
        self.rows = rows;
        self.base = above;
    }

    /// Padded with [`NBSP`]s to `width`, the text placed as `align` says.
    fn padded(&self, width: usize, align: Align) -> Pic {
        let cols = self.cols();
        let extra = width.saturating_sub(cols);
        let (left, right) = match align {
            Align::Left => (0, extra),
            Align::Right => (extra, 0),
            Align::Center => (extra / 2, extra - extra / 2),
        };
        let rows = (self.rows.iter())
            .map(|r| {
                let mut s = String::new();
                pad(&mut s, left);
                s.push_str(r);
                pad(&mut s, right);
                s
            })
            .collect();
        Pic {
            rows,
            base: self.base,
            width: cols.max(width),
        }
    }
}

fn pad(s: &mut String, n: usize) {
    s.extend(std::iter::repeat_n(NBSP, n));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// How a table's columns align.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColAlign {
    /// Display math's own lines: centered each, or aligned in pairs (`r l`) once a `&` is in
    /// them.
    Lines,
    /// `aligned` and its kin: right, left, right, left….
    Pairs,
    /// `gather`: one column, centered.
    Center,
    /// Matrices: every column centered, two columns between them.
    Matrix,
    /// `cases`: every column left, two columns between them.
    Cases,
    /// `array`: as its column spec says, up to eight.
    Spec([Align; 8], usize),
}

impl ColAlign {
    fn of_env(name: &str) -> Option<ColAlign> {
        Some(match name {
            "aligned" | "align" | "align*" | "split" | "alignat" | "alignat*" | "alignedat"
            | "flalign" | "flalign*" | "eqnarray" | "eqnarray*" => ColAlign::Pairs,
            "gather" | "gather*" | "gathered" | "equation" | "equation*" | "multline"
            | "multline*" | "displaylines" => ColAlign::Center,
            _ => return None,
        })
    }

    fn of(self, col: usize, any_cell: bool) -> Align {
        match self {
            ColAlign::Lines if !any_cell => Align::Center,
            ColAlign::Lines | ColAlign::Pairs if col.is_multiple_of(2) => Align::Right,
            ColAlign::Lines | ColAlign::Pairs | ColAlign::Cases => Align::Left,
            ColAlign::Center | ColAlign::Matrix => Align::Center,
            ColAlign::Spec(aligns, n) => aligns[col.min(n.saturating_sub(1)).min(7)],
        }
    }

    /// Columns between two of a row's cells.
    fn gap(self) -> usize {
        match self {
            ColAlign::Matrix | ColAlign::Cases | ColAlign::Spec(..) => 2,
            _ => 0,
        }
    }

    /// Whether a cell that isn't a row's first starts as after an atom: `&=` spaces its `=`
    /// as `x =` does, as TeX's `aligned` has it.
    fn joined(self) -> bool {
        matches!(self, ColAlign::Lines | ColAlign::Pairs)
    }
}

/// Rows of cells, each a [`Pic`].
struct Table {
    rows: Vec<Line>,
    /// Each row's height, once laid out.
    heights: Vec<usize>,
}

struct Line {
    cells: Vec<Pic>,
    /// The bytes of the text it was read from.
    tex: Range<usize>,
}

impl Table {
    /// The rows stacked, their columns aligned, the baseline on the middle row. Fills
    /// [`Table::heights`].
    fn layout(&mut self, align: ColAlign) -> Pic {
        let any_cell = self.rows.iter().any(|r| r.cells.len() > 1);
        let cols = self.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        let mut widths = vec![0; cols];
        for row in &self.rows {
            for (i, c) in row.cells.iter().enumerate() {
                widths[i] = widths[i].max(c.cols());
            }
        }
        let gap = align.gap();
        let mut rows = Vec::new();
        self.heights.clear();
        for row in &self.rows {
            let mut line = Pic::default();
            for (i, w) in widths.iter().enumerate() {
                let cell = row.cells.get(i).cloned().unwrap_or_default();
                let cell = match cell.rows.is_empty() {
                    true => Pic::line(String::new()),
                    false => cell,
                };
                if i > 0 && gap > 0 {
                    line = line.beside(Pic::line(NBSP.to_string().repeat(gap)));
                }
                line = line.beside(cell.padded(*w, align.of(i, any_cell)));
            }
            self.heights.push(line.rows.len());
            rows.append(&mut line.rows);
        }
        let width = rows.iter().map(|r| r.width()).max().unwrap_or(0);
        let rows: Vec<String> = (rows.into_iter())
            .map(|mut r| {
                let w = r.width();
                pad(&mut r, width - w);
                r
            })
            .collect();
        let base = rows.len().saturating_sub(1) / 2;
        Pic { rows, base, width }
    }
}

/// A sequence of atoms set on a line, or beside a tall piece: TeX's spacing between them.
struct Out {
    pic: Pic,
    line: String,
    /// The class of the last atom, `None` at the start.
    prev: Option<Class>,
    /// Spaces asked for before the next atom, by `\,`, `\quad` and the like.
    space: usize,
    /// The bytes of `line` the last atom took, when it was a fraction: one written next to it
    /// takes parentheses, `2(a/b)`, not `2a/b`.
    frac: Option<usize>,
    /// The last atom was a sign: `-` before a term.
    sign: bool,
    /// The last atom had scripts.
    scripted: bool,
    tight: bool,
}

impl Out {
    fn new(ctx: Ctx) -> Self {
        Self {
            pic: Pic::default(),
            line: String::new(),
            prev: None,
            space: 0,
            frac: None,
            sign: false,
            scripted: false,
            tight: ctx == Ctx::Script,
        }
    }

    /// Add an atom of `class`.
    fn push(&mut self, atom: Atom) {
        let mut class = atom.class;
        // a sign, not an operator, where nothing is on its left to operate on
        let sign = class == Class::Bin
            && matches!(
                self.prev,
                None | Some(Class::Bin | Class::Op | Class::Rel | Class::Open | Class::Punct)
            );
        if sign {
            class = Class::Ord;
        }
        let gap = match (self.prev, class) {
            (None, _) => 0,
            // a sign stands against what it signs: `-∑`
            _ if self.sign => 0,
            // a function's name keeps its space in a script too: `n_(c log₂ n)`
            (Some(Class::Op), Class::Ord | Class::Op)
            | (Some(Class::Ord | Class::Close), Class::Op) => 1,
            // limits written out keep a bracket off them: `max_(e∈C) {w(e)}`
            (Some(Class::Op), Class::Open) if self.scripted => 1,
            _ if self.tight => 0,
            (Some(Class::Rel), Class::Rel) => 0,
            (_, Class::Rel | Class::Bin) | (Some(Class::Rel | Class::Bin | Class::Punct), _) => 1,
            _ => 0,
        };
        let gap = match self.prev {
            None => 0,
            Some(_) => gap.max(self.space),
        };
        self.space = 0;
        self.sign = sign;
        self.scripted = atom.scripted;
        if gap > 0 {
            self.line.push(' ');
            pad(&mut self.line, gap - 1);
        }
        // a fraction beside a letter or a number takes parentheses
        if let Some(at) = self.frac.take()
            && gap == 0
            && atom.starts_tight()
        {
            self.line.insert(at, '(');
            self.line.push(')');
        }
        match atom.body {
            Body::Text(s) => {
                let after = self.line.chars().last();
                if atom.frac && gap == 0 && after.is_some_and(|c| tight(c) || c == ')') {
                    self.line.push('(');
                    self.line.push_str(&s);
                    self.line.push(')');
                } else {
                    if atom.frac {
                        self.frac = Some(self.line.len());
                    }
                    self.line.push_str(&s);
                }
            }
            Body::Pic(p) => {
                self.flush();
                self.pic = std::mem::take(&mut self.pic).beside(p);
            }
        }
        self.prev = Some(class);
    }

    /// The line so far, set beside the tall pieces before it.
    fn flush(&mut self) {
        if !self.line.is_empty() {
            let line = Pic::line(std::mem::take(&mut self.line));
            self.pic = std::mem::take(&mut self.pic).beside(line);
        }
        self.frac = None;
    }

    fn finish(mut self) -> Pic {
        self.flush();
        if self.pic.rows.is_empty() {
            return Pic::line(String::new());
        }
        self.pic
    }
}

/// Characters that bind to what's beside them: a fraction next to one takes parentheses, and a
/// part of a fraction made of them alone takes none.
fn tight(c: char) -> bool {
    c.is_alphanumeric()
        || is_combining(c)
        || matches!(
            c,
            '′' | '″'
                | '‴'
                | '√'
                | '∛'
                | '∜'
                | '∂'
                | '∇'
                | '∞'
                | '!'
                | '.'
                | '⁺'
                | '⁻'
                | '⁼'
                | '⁽'
                | '⁾'
                | '₊'
                | '₋'
                | '₌'
                | '₍'
                | '₎'
        )
}

fn is_combining(c: char) -> bool {
    matches!(c, '\u{300}'..='\u{36f}' | '\u{20d0}'..='\u{20ff}')
}

/// Whether `s` reads as one term: its characters [`tight`] but within brackets, or the whole
/// of it in one pair of them.
fn is_term(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' | '⟨' | '⌈' | '⌊' => depth += 1,
            ')' | ']' | '}' | '⟩' | '⌉' | '⌋' => depth -= 1,
            '|' | '‖' if depth > 0 => {}
            c if depth == 0 && !tight(c) => return false,
            _ => {}
        }
    }
    !s.is_empty()
}

/// A fraction's denominator or a root's radicand: in parentheses unless it is a single factor,
/// a number, a symbol with its scripts, a differential (`dx`, `∂x`), a function's value or a
/// root of one of them, so `a/(2b)` doesn't read as `(a/2)b`, nor `√(2π)` as `√2·π` (OBS-018).
fn factor(s: &str) -> String {
    // a root is one factor when what's under it is
    let s_all = s;
    let s = s
        .trim_start_matches(is_script)
        .trim_start_matches(['√', '∛', '∜']);
    let symbol = |t: &str| {
        let mut chars = t.chars();
        chars
            .next()
            .is_some_and(|c| !is_combining(c) && !is_script(c))
            && chars.all(|c| is_combining(c) || is_script(c) || matches!(c, '′' | '″' | '‴'))
    };
    let differential = s
        .strip_prefix(['d', '∂', 'δ', 'Δ'])
        .is_some_and(|r| symbol(r) || r.strip_suffix(['²', '³']).is_some_and(symbol));
    let number = s.chars().all(|c| c.is_ascii_digit() || c == '.');
    // a function's value: `f(x)`, `log(n)`
    let call = s
        .find('(')
        .is_some_and(|at| at > 0 && s[..at].chars().all(char::is_alphabetic) && wrapped(&s[at..]));
    if !s.is_empty() && (symbol(s) || differential || number || wrapped(s) || call) {
        s_all.to_owned()
    } else {
        format!("({s_all})")
    }
}

/// Whether `s` is one pair of brackets and what's in it.
fn wrapped(s: &str) -> bool {
    let (Some(first), Some(last)) = (s.chars().next(), s.chars().last()) else {
        return false;
    };
    let close = match first {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '⟨' => '⟩',
        _ => return false,
    };
    if last != close {
        return false;
    }
    // the first bracket closes at the end, not before
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if c == first {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + c.len_utf8() == s.len();
            }
        }
    }
    false
}

/// Unicode's sub- and superscript characters.
fn is_script(c: char) -> bool {
    matches!(c, '⁰'..='₟' | '¹' | '²' | '³' | 'ᵢ'..='ᵪ' | 'ᴬ'..='ᵡ' | 'ᶜ' | 'ᶠ' | 'ᶻ' | 'ᶿ' | 'ᶥ'
        | 'ʰ' | 'ʲ' | 'ʳ' | 'ʷ' | 'ʸ' | 'ˡ' | 'ˢ' | 'ˣ' | 'ⱼ' | 'ⱽ')
}

/// `s` as a term: in parentheses unless it is one.
fn term(s: &str) -> String {
    if is_term(s) {
        s.to_owned()
    } else {
        format!("({s})")
    }
}

#[derive(Debug)]
enum Body {
    Text(Cow<'static, str>),
    Pic(Pic),
}

#[derive(Debug)]
struct Atom {
    body: Body,
    class: Class,
    /// A fraction, written `a/b`: beside a term it takes parentheses.
    frac: bool,
    /// With scripts.
    scripted: bool,
}

impl Atom {
    fn text(s: impl Into<Cow<'static, str>>, class: Class) -> Self {
        Self {
            body: Body::Text(s.into()),
            class,
            frac: false,
            scripted: false,
        }
    }

    /// A piece as an atom: its text when it is a line.
    fn of(pic: Pic, class: Class) -> Self {
        if pic.is_line() {
            let s = pic.rows.into_iter().next().unwrap_or_default();
            return Atom::text(s, class);
        }
        Self {
            body: Body::Pic(pic),
            class,
            frac: false,
            scripted: false,
        }
    }

    fn starts_tight(&self) -> bool {
        match &self.body {
            Body::Text(s) => s.chars().next().is_some_and(tight),
            Body::Pic(_) => false,
        }
    }
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
    depth: u32,
    display: bool,
    /// Where the last row's separator, `\\` or `\cr`, starts.
    sep: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str, display: bool) -> Self {
        Self {
            s,
            i: 0,
            depth: 0,
            display,
            sep: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    /// Skip spaces and `%` comments, which math ignores.
    fn skip(&mut self) {
        loop {
            match self.s.as_bytes().get(self.i) {
                Some(b' ' | b'\n' | b'\t' | b'\r') => self.i += 1,
                Some(b) if b.is_ascii() && *b != b'%' => return,
                _ => {}
            }
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('%') => {
                    let line = self.rest().find('\n').map_or(self.s.len(), |n| self.i + n);
                    self.i = line;
                }
                _ => return,
            }
        }
    }

    /// The name of the control sequence at a `\`, taken: a word of letters, or one other
    /// character. Spaces after a word go with it.
    fn name(&mut self) -> Option<&'a str> {
        let at = self.i;
        if self.bump()? != '\\' {
            return None;
        }
        let start = self.i;
        let word = self
            .rest()
            .bytes()
            .take_while(u8::is_ascii_alphabetic)
            .count();
        if word > 0 {
            self.i += word;
            // `\operatorname*`
            if self.rest().starts_with('*') && &self.s[start..self.i] == "operatorname" {
                self.i += 1;
            }
            let name = &self.s[start..self.i];
            self.skip();
            return Some(name);
        }
        match self.bump() {
            Some(_) => Some(&self.s[start..self.i]),
            None => {
                self.i = at;
                None
            }
        }
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    /// Atoms up to `stop`, set in `ctx`; and what ended them. A row or a cell ends them only
    /// in [`Ctx::Cell`], else they are `None`.
    fn seq(&mut self, stop: Stop, ctx: Ctx) -> Option<(Out, Ended)> {
        self.seq_after(stop, ctx, None)
    }

    /// [`Parser::seq`], spaced as after an atom of class `prev`.
    fn seq_after(&mut self, stop: Stop, ctx: Ctx, prev: Option<Class>) -> Option<(Out, Ended)> {
        self.enter()?;
        let mut out = Out::new(ctx);
        out.prev = prev;
        let end = loop {
            self.skip();
            let Some(c) = self.peek() else {
                if stop == Stop::End {
                    break Ended::End;
                }
                return None;
            };
            match c {
                '}' if stop == Stop::Brace => {
                    self.bump();
                    break Ended::End;
                }
                ']' if stop == Stop::Bracket => {
                    self.bump();
                    break Ended::End;
                }
                '}' => return None,
                '&' if ctx == Ctx::Cell => {
                    self.bump();
                    break Ended::Cell;
                }
                '\\' if self.rest().starts_with("\\\\") || self.at_word("cr") => {
                    if ctx != Ctx::Cell {
                        return None;
                    }
                    self.sep = self.i;
                    if self.rest().starts_with("\\\\") {
                        self.i += 2;
                    } else {
                        self.name()?;
                    }
                    self.skip_spacing_arg();
                    break Ended::Row;
                }
                '\\' if self.at_word("end") => {
                    if stop != Stop::Env {
                        return None;
                    }
                    break Ended::Env;
                }
                '\\' if self.at_word("right") => {
                    if stop != Stop::Right {
                        return None;
                    }
                    break Ended::Right;
                }
                _ => {
                    let atom = self.atom(&mut out, ctx)?;
                    if let Some(atom) = atom {
                        out.push(atom);
                    }
                }
            }
        };
        self.depth -= 1;
        Some((out, end))
    }

    /// Whether the text goes on with `\word`, and no longer word.
    fn at_word(&self, word: &str) -> bool {
        let r = self.s.as_bytes().get(self.i..).unwrap_or_default();
        r.first() == Some(&b'\\')
            && r[1..].starts_with(word.as_bytes())
            && !r.get(1 + word.len()).is_some_and(u8::is_ascii_alphabetic)
    }

    /// `[2pt]` after a `\\`.
    fn skip_spacing_arg(&mut self) {
        let save = self.i;
        self.skip();
        if self.peek() == Some('[') {
            match self.rest().find(']') {
                Some(n) => self.i += n + 1,
                None => self.i = save,
            }
        } else {
            self.i = save;
        }
    }

    /// Rows of cells up to `stop`, each cell read as `align` sets it.
    fn table(&mut self, stop: Stop, align: ColAlign) -> Option<Table> {
        let mut rows = Vec::new();
        let mut cells = Vec::new();
        let mut row_start = self.i;
        loop {
            // a cell after a row's first starts as after an atom, when `joined`
            let prev = (!cells.is_empty() && align.joined()).then_some(Class::Ord);
            let (mut out, end) = self.seq_after(stop, Ctx::Cell, prev)?;
            out.space = 0;
            cells.push(out.finish());
            match end {
                Ended::Cell => {}
                Ended::Row | Ended::End | Ended::Env => {
                    let tex_end = match end {
                        Ended::Row => self.sep,
                        _ => self.i,
                    };
                    rows.push(Line {
                        cells: std::mem::take(&mut cells),
                        tex: row_start..tex_end,
                    });
                    row_start = self.i;
                    if end != Ended::Row {
                        break;
                    }
                }
                Ended::Right => return None,
            }
        }
        // a last row of nothing: the `\\` before `\end`
        if rows.len() > 1
            && rows.last().is_some_and(|r| {
                r.cells.len() == 1 && r.cells[0].rows.iter().all(|s| s.trim().is_empty())
            })
        {
            rows.pop();
        }
        Some(Table {
            rows,
            heights: Vec::new(),
        })
    }

    /// One atom with its scripts; `None` (the outer) for TeX past what this reads, `None`
    /// (the inner) for something that sets nothing (`\displaystyle`).
    fn atom(&mut self, out: &mut Out, ctx: Ctx) -> Option<Option<Atom>> {
        let Some(base) = self.base(out, ctx)? else {
            return Some(None);
        };
        self.scripts(base).map(Some)
    }

    /// The scripts after `base`: primes, `_`, `^`, in any order, once each.
    fn scripts(&mut self, mut base: Atom) -> Option<Atom> {
        let (mut sub, mut sup, mut primes) = (None, None, 0);
        loop {
            self.skip();
            let command = self.s.as_bytes().get(self.i) == Some(&b'\\');
            if command && (self.at_word("limits") || self.at_word("nolimits")) {
                self.name()?;
                continue;
            }
            match self.peek() {
                Some('\'') => {
                    self.bump();
                    primes += 1;
                }
                Some('_') if sub.is_none() => {
                    self.bump();
                    sub = Some(self.script_arg()?);
                }
                Some('^') if sup.is_none() => {
                    self.bump();
                    sup = Some(self.script_arg()?);
                }
                Some('_' | '^') => return None,
                _ => break,
            }
        }
        if sub.is_none() && sup.is_none() && primes == 0 {
            return Some(base);
        }
        base.scripted = true;
        let marks = match primes {
            0 => "",
            1 => "′",
            2 => "″",
            3 => "‴",
            _ => return None,
        };
        match &mut base.body {
            Body::Text(s) => {
                if base.frac {
                    *s = format!("({s})").into();
                    base.frac = false;
                }
                let s = s.to_mut();
                s.push_str(marks);
                if let Some(sub) = &sub {
                    s.push_str(&script(sub, sub_char, '_')?);
                }
                if let Some(sup) = &sup {
                    s.push_str(&script(sup, sup_char, '^')?);
                }
            }
            Body::Pic(p) => {
                // beside a tall piece: the superscript at its top, the subscript at its bottom
                let h = p.rows.len();
                let mut col = vec![String::new(); h];
                col[0].push_str(marks);
                if let Some(sup) = &sup {
                    col[0].push_str(&script(sup, sup_char, '^')?);
                }
                if let Some(sub) = &sub {
                    col[h - 1].push_str(&script(sub, sub_char, '_')?);
                }
                let width = col.iter().map(|r| r.width()).max().unwrap_or(0);
                let rows = (col.into_iter())
                    .map(|mut r| {
                        let w = r.width();
                        pad(&mut r, width - w);
                        r
                    })
                    .collect();
                let side = Pic {
                    rows,
                    base: p.base,
                    width,
                };
                *p = std::mem::take(p).beside(side);
            }
        }
        Some(base)
    }

    /// A script's argument: a group, a command with its arguments, or a character, set tight
    /// on one line.
    fn script_arg(&mut self) -> Option<String> {
        self.skip();
        line_of(self.arg_pic(Ctx::Script)?)
    }

    /// An argument in braces, or a single token, as a line.
    fn arg(&mut self, ctx: Ctx) -> Option<String> {
        line_of(self.arg_pic(ctx)?)
    }

    fn arg_pic(&mut self, ctx: Ctx) -> Option<Pic> {
        self.skip();
        let ctx = if ctx == Ctx::Script {
            Ctx::Script
        } else {
            Ctx::Line
        };
        if self.peek() == Some('{') {
            self.bump();
            return Some(self.seq(Stop::Brace, ctx)?.0.finish());
        }
        // a single token: `\frac12`, `x_\alpha`
        let mut out = Out::new(ctx);
        if let Some(base) = self.base(&mut out, ctx)? {
            out.push(base);
        }
        Some(out.finish())
    }

    /// The text of a `{…}` argument as written, for names (`\begin{pmatrix}`, `\color{red}`).
    fn raw_arg(&mut self) -> Option<&'a str> {
        self.skip();
        if self.bump()? != '{' {
            return None;
        }
        let start = self.i;
        let n = self.rest().find('}')?;
        self.i += n + 1;
        Some(&self.s[start..start + n])
    }

    /// `\text{…}`: its words as written, spaces and all; `$` and commands but escapes are
    /// `None`.
    fn text_arg(&mut self) -> Option<String> {
        self.skip();
        if self.bump()? != '{' {
            return None;
        }
        let mut s = String::new();
        let mut depth = 0;
        loop {
            match self.bump()? {
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                '{' => depth += 1,
                '\\' => match self.bump()? {
                    c @ ('{' | '}' | '%' | '&' | '_' | '#' | '$' | ' ') => s.push(c),
                    '\\' => return None,
                    _ => return None,
                },
                '$' => return None,
                '~' => s.push(' '),
                c if c.is_whitespace() => {
                    if !s.ends_with(' ') {
                        s.push(' ');
                    }
                }
                c => s.push(c),
            }
        }
        Some(s)
    }

    /// The atom at the cursor, without scripts. `Some(None)`: a command that sets nothing.
    fn base(&mut self, out: &mut Out, ctx: Ctx) -> Option<Option<Atom>> {
        let c = self.peek()?;
        if c == '\\' {
            return self.command(out, ctx);
        }
        if c == '{' {
            self.bump();
            let pic = self.seq(Stop::Brace, ctx_inner(ctx))?.0.finish();
            return Some(Some(Atom::of(pic, Class::Ord)));
        }
        // a script with nothing before it: `^{14}C`
        if c == '_' || c == '^' {
            return Some(Some(Atom::text("", Class::Ord)));
        }
        self.bump();
        let class = match c {
            '+' | '-' | '*' => Class::Bin,
            '=' | '<' | '>' | ':' => Class::Rel,
            ',' | ';' => Class::Punct,
            '(' | '[' => Class::Open,
            ')' | ']' | '!' | '?' => Class::Close,
            '~' => {
                out.space += 1;
                return Some(None);
            }
            '&' | '#' | '$' | '}' => return None,
            '\'' => return Some(Some(Atom::text("′", Class::Ord))),
            c if c.is_control() => return None,
            _ => Class::Ord,
        };
        let s: Cow<'static, str> = match c {
            '*' => "∗".into(),
            ' '..='~' => {
                let at = c as usize - ' ' as usize;
                PRINTABLE[at..at + 1].into()
            }
            c => c.to_string().into(),
        };
        Some(Some(Atom::text(s, class)))
    }

    /// A command and its arguments, counted toward [`MAX_DEPTH`] as a group is: an argument
    /// without braces (`\sqrt\sqrt x`, `x^\hat\hat y`, `\not\not`) nests through here, never
    /// through [`Parser::seq_after`], and a chain of them would otherwise overflow the stack.
    fn command(&mut self, out: &mut Out, ctx: Ctx) -> Option<Option<Atom>> {
        self.depth += 1;
        let atom = match self.depth <= MAX_DEPTH {
            true => self.command_args(out, ctx),
            false => None,
        };
        self.depth -= 1;
        atom
    }

    fn command_args(&mut self, out: &mut Out, ctx: Ctx) -> Option<Option<Atom>> {
        let name = self.name()?;
        if let Some((s, class)) = symbol(name) {
            return Some(Some(Atom::text(s, class)));
        }
        if let Some(&f) = FUNCTIONS.iter().find(|&&f| f == name) {
            return Some(Some(Atom::text(f, Class::Op)));
        }
        let atom = match name {
            // spacing
            "," | ":" | ";" | ">" | "thinspace" | "medspace" | "thickspace" | "enspace" => {
                out.space = out.space.max(1);
                return Some(None);
            }
            // spaces of a word's width and wider add up: `\ \ \` is three
            " " | "space" => {
                out.space += 1;
                return Some(None);
            }
            "quad" => {
                out.space += 2;
                return Some(None);
            }
            "qquad" => {
                out.space += 4;
                return Some(None);
            }
            "tiny" | "scriptsize" | "footnotesize" | "small" | "normalsize" | "large" | "Large"
            | "LARGE" | "huge" | "Huge" => return Some(None),
            "hspace" | "hskip" | "kern" => {
                if name == "hspace" {
                    self.raw_arg()?;
                } else {
                    return None;
                }
                out.space = out.space.max(1);
                return Some(None);
            }
            "!" | "negthinspace" | "displaystyle" | "textstyle" | "scriptstyle"
            | "scriptscriptstyle" | "limits" | "nolimits" | "nonumber" | "notag" | "big"
            | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "bigm" | "Bigl" | "Bigr" | "Bigm"
            | "biggl" | "biggr" | "biggm" | "Biggl" | "Biggr" | "Biggm" | "hline" | "middle" => {
                return Some(None);
            }
            "label" => {
                self.raw_arg()?;
                return Some(None);
            }
            "color" => {
                self.raw_arg()?;
                return Some(None);
            }
            "tag" | "tag*" => {
                let t = self.text_arg()?;
                out.space = out.space.max(4);
                return Some(Some(Atom::text(format!("({t})"), Class::Ord)));
            }
            // fractions and roots
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = self.arg(ctx)?;
                let b = self.arg(ctx)?;
                let s = format!("{}/{}", term(&a), factor(&b));
                return Some(Some(Atom {
                    body: Body::Text(s.into()),
                    class: Class::Ord,
                    frac: true,
                    scripted: false,
                }));
            }
            "sqrt" => {
                self.skip();
                let root = if self.peek() == Some('[') {
                    self.bump();
                    let (o, _) = self.seq(Stop::Bracket, Ctx::Script)?;
                    Some(line_of(o.finish())?)
                } else {
                    None
                };
                let x = self.arg(ctx)?;
                let sign = match root.as_deref() {
                    None | Some("2") => "√".to_owned(),
                    Some("3") => "∛".to_owned(),
                    Some("4") => "∜".to_owned(),
                    Some(n) => format!("{}√", script(n, sup_char, '^')?),
                };
                Atom::text(format!("{sign}{}", factor(&x)), Class::Ord)
            }
            // text and alphabets
            "text" | "textrm" | "textnormal" | "textit" | "textbf" | "textsf" | "texttt"
            | "mbox" | "hbox" | "emph" | "textup" => {
                let t = self.text_arg()?;
                let lead = t.starts_with(' ');
                let trail = t.ends_with(' ');
                let t = t.trim();
                if lead {
                    out.space = out.space.max(1);
                }
                if t.is_empty() {
                    if trail {
                        out.space = out.space.max(1);
                    }
                    return Some(None);
                }
                out.push(Atom::text(t.to_owned(), Class::Ord));
                if trail {
                    out.space = out.space.max(1);
                }
                return Some(None);
            }
            "mathrm" | "mathit" | "mathsf" | "mathtt" | "mathnormal" | "boxed" | "mathop"
            | "mathord" | "textcolor" | "mathrel" | "mathbin" => {
                if name == "textcolor" {
                    self.raw_arg()?;
                }
                let class = match name {
                    "mathop" => Class::Op,
                    "mathrel" => Class::Rel,
                    "mathbin" => Class::Bin,
                    _ => Class::Ord,
                };
                let pic = self.arg_pic(ctx)?;
                return Some(Some(Atom::of(pic, class)));
            }
            "operatorname" | "operatorname*" => {
                let t = self.arg(ctx)?;
                Atom::text(t, Class::Op)
            }
            "mathbb" | "Bbb" | "mathcal" | "mathscr" | "mathfrak" | "mathbf" | "boldsymbol"
            | "bm" | "pmb" | "mathbfit" => {
                let t = self.arg(ctx)?;
                let f = match name {
                    "mathbb" | "Bbb" => double_struck,
                    "mathcal" | "mathscr" => script_letter,
                    "mathfrak" => fraktur,
                    _ => bold,
                };
                Atom::text(t.chars().map(f).collect::<String>(), Class::Ord)
            }
            // accents
            "hat" | "widehat" | "tilde" | "widetilde" | "bar" | "overline" | "vec"
            | "overrightarrow" | "dot" | "ddot" | "check" | "breve" | "acute" | "grave"
            | "underline" | "mathring" => {
                let t = self.arg(ctx)?;
                let mark = match name {
                    "hat" | "widehat" => '\u{302}',
                    "tilde" | "widetilde" => '\u{303}',
                    "bar" => '\u{304}',
                    "overline" => '\u{305}',
                    "vec" | "overrightarrow" => '\u{20d7}',
                    "dot" => '\u{307}',
                    "ddot" => '\u{308}',
                    "check" => '\u{30c}',
                    "breve" => '\u{306}',
                    "acute" => '\u{301}',
                    "grave" => '\u{300}',
                    "mathring" => '\u{30a}',
                    _ => '\u{332}',
                };
                Atom::text(accent(&t, mark, name)?, Class::Ord)
            }
            "not" => {
                self.skip();
                let save = self.i;
                let next = match self.peek()? {
                    '\\' => self.name()?.to_owned(),
                    c => {
                        self.bump();
                        c.to_string()
                    }
                };
                let neg = match next.as_str() {
                    "=" => "≠",
                    "<" => "≮",
                    ">" => "≯",
                    "in" => "∉",
                    "ni" => "∌",
                    "subset" => "⊄",
                    "supset" => "⊅",
                    "subseteq" => "⊈",
                    "supseteq" => "⊉",
                    "equiv" => "≢",
                    "mid" => "∤",
                    "parallel" => "∦",
                    "exists" => "∄",
                    "le" | "leq" => "≰",
                    "ge" | "geq" => "≱",
                    "sim" => "≁",
                    "approx" => "≉",
                    "cong" => "≇",
                    "simeq" => "≄",
                    _ => {
                        self.i = save;
                        let atom = self.base(out, ctx)??;
                        let Body::Text(s) = atom.body else {
                            return None;
                        };
                        return Some(Some(Atom::text(format!("{s}\u{338}"), atom.class)));
                    }
                };
                Atom::text(neg, Class::Rel)
            }
            // annotations set as scripts: `\overset{def}{=}` as =^(def)
            "overset" | "stackrel" | "underset" => {
                let over = self.arg(Ctx::Script)?;
                let base = self.arg(ctx)?;
                let class = first_class(&base);
                let s = if name == "underset" {
                    format!("{base}{}", script(&over, sub_char, '_')?)
                } else {
                    format!("{base}{}", script(&over, sup_char, '^')?)
                };
                Atom::text(s, class)
            }
            "underbrace" | "overbrace" => {
                let t = self.arg(ctx)?;
                Atom::text(t, Class::Ord)
            }
            "xrightarrow" | "xleftarrow" => {
                self.skip();
                if self.peek() == Some('[') {
                    return None;
                }
                let t = self.arg(Ctx::Script)?;
                let arrow = if name == "xrightarrow" { "→" } else { "←" };
                Atom::text(format!("{arrow}{}", script(&t, sup_char, '^')?), Class::Rel)
            }
            "pmod" => {
                let t = self.arg(ctx)?;
                out.space = out.space.max(1);
                Atom::text(format!("(mod {t})"), Class::Ord)
            }
            "bmod" | "mod" => Atom::text("mod", Class::Bin),
            // delimiters
            "left" => return self.left_right().map(Some),
            "binom" | "dbinom" | "tbinom" => {
                let n = self.arg_pic(ctx)?;
                let k = self.arg_pic(ctx)?;
                if !self.display {
                    return None;
                }
                let line = |p| Line {
                    cells: vec![p],
                    tex: 0..0,
                };
                let mut t = Table {
                    rows: vec![line(n), line(k)],
                    heights: Vec::new(),
                };
                let pic = bracketed("(", t.layout(ColAlign::Matrix), ")")?;
                return Some(Some(Atom {
                    body: Body::Pic(pic),
                    class: Class::Ord,
                    frac: false,
                    scripted: false,
                }));
            }
            "begin" => {
                if ctx == Ctx::Script {
                    return None;
                }
                let env = self.raw_arg()?;
                return Some(Some(self.env(env)?));
            }
            _ => return None,
        };
        Some(Some(atom))
    }

    /// `\left( … \right)`: the delimiters as tall as what they hold.
    fn left_right(&mut self) -> Option<Atom> {
        let open = self.delimiter()?;
        let (inner, end) = self.seq(Stop::Right, Ctx::Line)?;
        if end != Ended::Right {
            return None;
        }
        self.name()?; // `right`
        let close = self.delimiter()?;
        let inner = inner.finish();
        if inner.is_line() {
            let s = inner.rows.into_iter().next().unwrap_or_default();
            return Some(Atom::text(format!("{open}{s}{close}"), Class::Ord));
        }
        Some(Atom {
            body: Body::Pic(bracketed(open, inner, close)?),
            class: Class::Ord,
            frac: false,
            scripted: false,
        })
    }

    /// A delimiter after `\left`, `\right`: `""` for `.`.
    fn delimiter(&mut self) -> Option<&'static str> {
        self.skip();
        let d = match self.bump()? {
            '.' => "",
            '(' => "(",
            ')' => ")",
            '[' => "[",
            ']' => "]",
            '|' => "|",
            '/' => "/",
            '<' => "⟨",
            '>' => "⟩",
            '\\' => {
                self.i -= 1;
                match self.name()? {
                    "{" | "lbrace" => "{",
                    "}" | "rbrace" => "}",
                    "|" | "Vert" | "lVert" | "rVert" => "‖",
                    "vert" | "lvert" | "rvert" => "|",
                    "langle" => "⟨",
                    "rangle" => "⟩",
                    "lceil" => "⌈",
                    "rceil" => "⌉",
                    "lfloor" => "⌊",
                    "rfloor" => "⌋",
                    "lbrack" => "[",
                    "rbrack" => "]",
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(d)
    }

    /// `\begin{env}`, its name read: a matrix, cases or rows of their own, as a tall piece.
    fn env(&mut self, name: &str) -> Option<Atom> {
        let (align, open, close) = match name {
            "matrix" | "smallmatrix" => (ColAlign::Matrix, "", ""),
            "pmatrix" => (ColAlign::Matrix, "(", ")"),
            "bmatrix" => (ColAlign::Matrix, "[", "]"),
            "Bmatrix" => (ColAlign::Matrix, "{", "}"),
            "vmatrix" => (ColAlign::Matrix, "|", "|"),
            "Vmatrix" => (ColAlign::Matrix, "‖", "‖"),
            "cases" | "dcases" | "cases*" => (ColAlign::Cases, "{", ""),
            "rcases" => (ColAlign::Cases, "", "}"),
            "array" => {
                let spec = self.raw_arg()?;
                let mut aligns = [Align::Center; 8];
                let mut n = 0;
                for c in spec.chars() {
                    let a = match c {
                        'l' => Align::Left,
                        'c' => Align::Center,
                        'r' => Align::Right,
                        '|' | ' ' => continue,
                        _ => return None,
                    };
                    if n < 8 {
                        aligns[n] = a;
                    }
                    n += 1;
                }
                (ColAlign::Spec(aligns, n), "", "")
            }
            _ => (ColAlign::of_env(name)?, "", ""),
        };
        let mut table = self.table(Stop::Env, align)?;
        // `\end{name}`
        self.name()?;
        if self.raw_arg()? != name {
            return None;
        }
        if !self.display {
            // inline, a matrix is written on its line: `[a b; c d]`
            if align != ColAlign::Matrix {
                return None;
            }
            let mut rows = Vec::new();
            for row in table.rows {
                let cells: Option<Vec<String>> = row.cells.into_iter().map(line_of).collect();
                rows.push(cells?.join(" "));
            }
            let s = format!("{open}{}{close}", rows.join("; "));
            return Some(Atom::text(s, Class::Ord));
        }
        let pic = bracketed(open, table.layout(align), close)?;
        if pic.is_line() {
            let s = pic.rows.into_iter().next().unwrap_or_default();
            return Some(Atom::text(s.replace(NBSP, " "), Class::Ord));
        }
        Some(Atom {
            body: Body::Pic(pic),
            class: Class::Ord,
            frac: false,
            scripted: false,
        })
    }
}

/// `inner` between delimiters `open` and `close` as tall as it is (`""` for none).
fn bracketed(open: &str, inner: Pic, close: &str) -> Option<Pic> {
    let (h, base) = (inner.rows.len(), inner.base);
    let side = |d: &str| {
        let mut p = tall(d, h)?;
        p.base = base;
        Some(p)
    };
    Some(side(open)?.beside(inner).beside(side(close)?))
}

fn ctx_inner(ctx: Ctx) -> Ctx {
    match ctx {
        Ctx::Script => Ctx::Script,
        _ => Ctx::Line,
    }
}

fn line_of(pic: Pic) -> Option<String> {
    if pic.is_line() {
        pic.rows.into_iter().next()
    } else {
        None
    }
}

/// The class of a run of text by its first character, for one read whole: `=` is a relation.
fn first_class(s: &str) -> Class {
    match s.chars().next() {
        Some('=' | '<' | '>') => Class::Rel,
        _ => Class::Ord,
    }
}

/// A delimiter `h` rows tall, its pieces from Unicode's bracket parts; `""` is none, no columns
/// wide.
fn tall(d: &str, h: usize) -> Option<Pic> {
    let h = h.max(1);
    if h == 1 || d.is_empty() {
        return Some(Pic {
            rows: vec![d.to_owned(); h],
            base: (h - 1) / 2,
            width: d.width(),
        });
    }
    let (top, mid, bottom, centre) = match d {
        "(" => ('⎛', '⎜', '⎝', None),
        ")" => ('⎞', '⎟', '⎠', None),
        "[" => ('⎡', '⎢', '⎣', None),
        "]" => ('⎤', '⎥', '⎦', None),
        "{" if h == 2 => ('⎰', '⎪', '⎱', None),
        "}" if h == 2 => ('⎱', '⎪', '⎰', None),
        "{" => ('⎧', '⎪', '⎩', Some('⎨')),
        "}" => ('⎫', '⎪', '⎭', Some('⎬')),
        "|" => ('│', '│', '│', None),
        "‖" => ('‖', '‖', '‖', None),
        "⌈" => ('⎡', '⎢', '⎢', None),
        "⌉" => ('⎤', '⎥', '⎥', None),
        "⌊" => ('⎢', '⎢', '⎣', None),
        "⌋" => ('⎥', '⎥', '⎦', None),
        _ => return None,
    };
    let rows = (0..h)
        .map(|i| match i {
            0 => top,
            i if i == h - 1 => bottom,
            i if i == (h - 1) / 2 => centre.unwrap_or(mid),
            _ => mid,
        })
        .map(String::from)
        .collect();
    Some(Pic {
        rows,
        base: (h - 1) / 2,
        width: 1,
    })
}

/// `s` with accent `mark` over it: a character's, or for `\overline` and `\underline` each
/// one's.
fn accent(s: &str, mark: char, name: &str) -> Option<String> {
    // a character with its own marks, then only scripts: `\vec{u_1}` is u⃗₁
    let base = (s.char_indices().skip(1))
        .find(|(_, c)| !is_combining(*c))
        .map_or(s.len(), |(i, _)| i);
    let rest = &s[base..];
    if base > 0
        && rest
            .chars()
            .all(|c| is_script(c) || matches!(c, '′' | '″' | '‴'))
    {
        return Some(format!("{}{mark}{rest}", &s[..base]));
    }
    if matches!(name, "overline" | "underline" | "bar") && !s.is_empty() {
        let mark = if name == "bar" { '\u{305}' } else { mark };
        let mut out = String::new();
        for c in s.chars() {
            out.push(c);
            if !is_combining(c) && c != ' ' {
                out.push(mark);
            }
        }
        return Some(out);
    }
    None
}

/// A script after its base: in Unicode's script letters when `map` has every one, else
/// written out after `mark`, in parentheses when it's more than one character. The marks that
/// have no script letter and sit low ([`low`]: `x^*`, `A^\dagger`) are taken only beside other
/// such marks ([`as_is`]): among script letters they'd sit on the baseline, so `5^{10*13}` is
/// written out (OBS-019). Primes and the degree sign sit high, so `e^{i\theta'}` stays `eⁱᶿ′`.
fn script(s: &str, map: fn(char) -> Option<char>, mark: char) -> Option<String> {
    if s.is_empty() {
        return Some(String::new());
    }
    if let Some(mapped) = s.chars().map(map).collect::<Option<String>>()
        && (mapped.chars().all(as_is) || !mapped.chars().any(low))
    {
        return Some(mapped);
    }
    let mut chars = s.chars().filter(|c| !is_combining(*c));
    let one = chars.next().is_some() && chars.next().is_none();
    Some(if one {
        format!("{mark}{s}")
    } else {
        format!("{mark}({s})")
    })
}

/// The marks a superscript keeps as written, having no script letter: primes, stars, daggers
/// and the degree sign (`\circ`'s).
fn as_is(c: char) -> bool {
    matches!(c, '′' | '″' | '‴' | '°') || low(c)
}

/// The marks of [`as_is`] that sit low, as no script letter does: stars and daggers.
fn low(c: char) -> bool {
    matches!(c, '∗' | '*' | '†' | '‡')
}

fn sub_char(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' => char::from_u32(0x2080 + (c as u32 - '0' as u32))?,
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'o' => 'ₒ',
        'x' => 'ₓ',
        'h' => 'ₕ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'p' => 'ₚ',
        's' => 'ₛ',
        't' => 'ₜ',
        'i' => 'ᵢ',
        'r' => 'ᵣ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'j' => 'ⱼ',
        'β' => 'ᵦ',
        'γ' => 'ᵧ',
        'ρ' => 'ᵨ',
        'φ' => 'ᵩ',
        'χ' => 'ᵪ',
        ',' => ',',
        _ => return None,
    })
}

fn sup_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4'..='9' => char::from_u32(0x2074 + (c as u32 - '4' as u32))?,
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' | '⊤' => 'ᵀ',
        '/' => '⁄',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        'α' => 'ᵅ',
        'β' => 'ᵝ',
        'γ' => 'ᵞ',
        'δ' => 'ᵟ',
        'ε' => 'ᵋ',
        'θ' => 'ᶿ',
        'ι' => 'ᶥ',
        'φ' => 'ᵠ',
        'χ' => 'ᵡ',
        '∘' => '°',
        c if as_is(c) => c,
        _ => return None,
    })
}

/// A letter in a math alphabet of Unicode's: the block's letter at `upper` or `lower` plus its
/// place, but where `holes` give the letter its older code point.
fn alphabet(c: char, upper: u32, lower: u32, digits: Option<u32>, holes: &[(char, char)]) -> char {
    if let Some(&(_, h)) = holes.iter().find(|(k, _)| *k == c) {
        return h;
    }
    let at = match c {
        'A'..='Z' => upper + (c as u32 - 'A' as u32),
        'a'..='z' => lower + (c as u32 - 'a' as u32),
        '0'..='9' => match digits {
            Some(d) => d + (c as u32 - '0' as u32),
            None => return c,
        },
        _ => return c,
    };
    char::from_u32(at).unwrap_or(c)
}

fn double_struck(c: char) -> char {
    const HOLES: &[(char, char)] = &[
        ('C', 'ℂ'),
        ('H', 'ℍ'),
        ('N', 'ℕ'),
        ('P', 'ℙ'),
        ('Q', 'ℚ'),
        ('R', 'ℝ'),
        ('Z', 'ℤ'),
    ];
    alphabet(c, 0x1d538, 0x1d552, Some(0x1d7d8), HOLES)
}

fn script_letter(c: char) -> char {
    const HOLES: &[(char, char)] = &[
        ('B', 'ℬ'),
        ('E', 'ℰ'),
        ('F', 'ℱ'),
        ('H', 'ℋ'),
        ('I', 'ℐ'),
        ('L', 'ℒ'),
        ('M', 'ℳ'),
        ('R', 'ℛ'),
        ('e', 'ℯ'),
        ('g', 'ℊ'),
        ('o', 'ℴ'),
    ];
    alphabet(c, 0x1d49c, 0x1d4b6, None, HOLES)
}

fn fraktur(c: char) -> char {
    const HOLES: &[(char, char)] = &[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')];
    alphabet(c, 0x1d504, 0x1d51e, None, HOLES)
}

fn bold(c: char) -> char {
    alphabet(c, 0x1d400, 0x1d41a, Some(0x1d7ce), &[])
}

/// ASCII's printable characters, for an atom of one without an allocation.
const PRINTABLE: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// Function names, set upright as words: `\sin x` is `sin x`.
const FUNCTIONS: &[&str] = &[
    "arccos", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "deg", "det", "dim",
    "exp", "gcd", "hom", "inf", "ker", "lg", "lim", "liminf", "limsup", "ln", "log", "max", "min",
    "Pr", "sec", "sin", "sinh", "sup", "tan", "tanh", "lcm", "sgn", "arccot", "sech", "csch",
];

/// `\name`'s character and class, if it's a symbol of [`SYMBOLS`] or a Greek letter (`\up…` of
/// upgreek too).
fn symbol(name: &str) -> Option<(&'static str, Class)> {
    if let Ok(i) = SYMBOLS.binary_search_by(|(n, ..)| n.cmp(&name)) {
        let (_, s, class) = SYMBOLS[i];
        return Some((s, class));
    }
    let greek = name.strip_prefix("up").unwrap_or(name);
    GREEK
        .binary_search_by(|(n, _)| n.cmp(&greek))
        .ok()
        .map(|i| (GREEK[i].1, Class::Ord))
}

/// Greek letters, sorted by name.
const GREEK: &[(&str, &str)] = &[
    ("Delta", "Δ"),
    ("Gamma", "Γ"),
    ("Lambda", "Λ"),
    ("Omega", "Ω"),
    ("Phi", "Φ"),
    ("Pi", "Π"),
    ("Psi", "Ψ"),
    ("Sigma", "Σ"),
    ("Theta", "Θ"),
    ("Upsilon", "Υ"),
    ("Xi", "Ξ"),
    ("alpha", "α"),
    ("beta", "β"),
    ("chi", "χ"),
    ("delta", "δ"),
    ("digamma", "ϝ"),
    ("epsilon", "ϵ"),
    ("eta", "η"),
    ("gamma", "γ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("omega", "ω"),
    ("omicron", "ο"),
    ("phi", "ϕ"),
    ("pi", "π"),
    ("psi", "ψ"),
    ("rho", "ρ"),
    ("sigma", "σ"),
    ("tau", "τ"),
    ("theta", "θ"),
    ("upsilon", "υ"),
    ("varepsilon", "ε"),
    ("varkappa", "ϰ"),
    ("varphi", "φ"),
    ("varpi", "ϖ"),
    ("varrho", "ϱ"),
    ("varsigma", "ς"),
    ("vartheta", "ϑ"),
    ("xi", "ξ"),
    ("zeta", "ζ"),
];

use Class::{Bin, Close, Op, Open, Ord, Punct, Rel};

/// Commands that are a symbol, sorted by name (a test checks): the character and its class.
const SYMBOLS: &[(&str, &str, Class)] = &[
    ("#", "#", Ord),
    ("$", "$", Ord),
    ("%", "%", Ord),
    ("&", "&", Ord),
    ("And", "&", Ord),
    ("Box", "□", Ord),
    ("Downarrow", "⇓", Rel),
    ("Im", "ℑ", Ord),
    ("Join", "⋈", Bin),
    ("Leftarrow", "⇐", Rel),
    ("Leftrightarrow", "⇔", Rel),
    ("Longleftarrow", "⟸", Rel),
    ("Longleftrightarrow", "⟺", Rel),
    ("Longrightarrow", "⟹", Rel),
    ("Re", "ℜ", Ord),
    ("Rightarrow", "⇒", Rel),
    ("Uparrow", "⇑", Rel),
    ("Vert", "‖", Ord),
    ("_", "_", Ord),
    ("aleph", "ℵ", Ord),
    ("amalg", "⨿", Bin),
    ("angle", "∠", Ord),
    ("approx", "≈", Rel),
    ("approxeq", "≊", Rel),
    ("ast", "∗", Bin),
    ("asymp", "≍", Rel),
    ("backslash", "\\", Ord),
    ("because", "∵", Rel),
    ("bigcap", "⋂", Op),
    ("bigcirc", "◯", Bin),
    ("bigcup", "⋃", Op),
    ("bigodot", "⨀", Op),
    ("bigoplus", "⨁", Op),
    ("bigotimes", "⨂", Op),
    ("bigsqcup", "⨆", Op),
    ("bigstar", "★", Ord),
    ("bigtriangledown", "▽", Bin),
    ("bigtriangleup", "△", Bin),
    ("biguplus", "⨄", Op),
    ("bigvee", "⋁", Op),
    ("bigwedge", "⋀", Op),
    ("blacksquare", "■", Ord),
    ("blacktriangle", "▲", Ord),
    ("blacktriangledown", "▼", Ord),
    ("blacktriangleleft", "◀", Rel),
    ("blacktriangleright", "▶", Rel),
    ("bot", "⊥", Ord),
    ("bowtie", "⋈", Rel),
    ("bullet", "∙", Bin),
    ("cap", "∩", Bin),
    ("cdot", "·", Bin),
    ("cdotp", "·", Punct),
    ("cdots", "⋯", Ord),
    ("checkmark", "✓", Ord),
    ("circ", "∘", Bin),
    ("circledast", "⊛", Bin),
    ("clubsuit", "♣", Ord),
    ("colon", ":", Punct),
    ("coloneqq", "≔", Rel),
    ("complement", "∁", Ord),
    ("cong", "≅", Rel),
    ("coprod", "∐", Op),
    ("cup", "∪", Bin),
    ("dagger", "†", Bin),
    ("dashv", "⊣", Rel),
    ("ddagger", "‡", Bin),
    ("ddots", "⋱", Ord),
    ("degree", "°", Ord),
    ("diamond", "⋄", Bin),
    ("diamondsuit", "♢", Ord),
    ("div", "÷", Bin),
    ("doteq", "≐", Rel),
    ("dots", "…", Ord),
    ("dotsb", "⋯", Ord),
    ("dotsc", "…", Ord),
    ("dotsi", "⋯", Ord),
    ("dotsm", "⋯", Ord),
    ("downarrow", "↓", Rel),
    ("ell", "ℓ", Ord),
    ("emptyset", "∅", Ord),
    ("eqcirc", "≖", Rel),
    ("equiv", "≡", Rel),
    ("exists", "∃", Ord),
    ("flat", "♭", Ord),
    ("forall", "∀", Ord),
    ("frown", "⌢", Rel),
    ("ge", "≥", Rel),
    ("geq", "≥", Rel),
    ("geqq", "≧", Rel),
    ("geqslant", "⩾", Rel),
    ("gets", "←", Rel),
    ("gg", "≫", Rel),
    ("gt", ">", Rel),
    ("gtrdot", "⋗", Rel),
    ("gtrsim", "≳", Rel),
    ("hbar", "ℏ", Ord),
    ("heartsuit", "♡", Ord),
    ("hookleftarrow", "↩", Rel),
    ("hookrightarrow", "↪", Rel),
    ("iff", "⟺", Rel),
    ("iiint", "∭", Op),
    ("iint", "∬", Op),
    ("imath", "ı", Ord),
    ("impliedby", "⟸", Rel),
    ("implies", "⟹", Rel),
    ("in", "∈", Rel),
    ("infty", "∞", Ord),
    ("int", "∫", Op),
    ("intercal", "⊺", Bin),
    ("jmath", "ȷ", Ord),
    ("lVert", "‖", Open),
    ("land", "∧", Bin),
    ("langle", "⟨", Open),
    ("lbrace", "{", Open),
    ("lbrack", "[", Open),
    ("lceil", "⌈", Open),
    ("ldots", "…", Ord),
    ("le", "≤", Rel),
    ("leadsto", "⇝", Rel),
    ("leftarrow", "←", Rel),
    ("leftharpoonup", "↼", Rel),
    ("leftrightarrow", "↔", Rel),
    ("leftrightarrows", "⇆", Rel),
    ("leq", "≤", Rel),
    ("leqq", "≦", Rel),
    ("leqslant", "⩽", Rel),
    ("lessdot", "⋖", Rel),
    ("lesssim", "≲", Rel),
    ("lfloor", "⌊", Open),
    ("lhd", "⊲", Bin),
    ("ll", "≪", Rel),
    ("lnot", "¬", Ord),
    ("longleftarrow", "⟵", Rel),
    ("longleftrightarrow", "⟷", Rel),
    ("longmapsto", "⟼", Rel),
    ("longrightarrow", "⟶", Rel),
    ("lor", "∨", Bin),
    ("lozenge", "◊", Ord),
    ("lt", "<", Rel),
    ("lvert", "|", Open),
    ("mapsto", "↦", Rel),
    ("measuredangle", "∡", Ord),
    ("mid", "∣", Rel),
    ("models", "⊨", Rel),
    ("mp", "∓", Bin),
    ("nRightarrow", "⇏", Rel),
    ("nabla", "∇", Ord),
    ("natural", "♮", Ord),
    ("ncong", "≇", Rel),
    ("ne", "≠", Rel),
    ("nearrow", "↗", Rel),
    ("neg", "¬", Ord),
    ("neq", "≠", Rel),
    ("nexists", "∄", Ord),
    ("ngeq", "≱", Rel),
    ("ni", "∋", Rel),
    ("nleq", "≰", Rel),
    ("nmid", "∤", Rel),
    ("notin", "∉", Rel),
    ("nparallel", "∦", Rel),
    ("nrightarrow", "↛", Rel),
    ("nsim", "≁", Rel),
    ("nsubseteq", "⊈", Rel),
    ("nwarrow", "↖", Rel),
    ("odot", "⊙", Bin),
    ("oint", "∮", Op),
    ("ominus", "⊖", Bin),
    ("oplus", "⊕", Bin),
    ("oslash", "⊘", Bin),
    ("otimes", "⊗", Bin),
    ("parallel", "∥", Rel),
    ("partial", "∂", Ord),
    ("perp", "⊥", Rel),
    ("pm", "±", Bin),
    ("prec", "≺", Rel),
    ("preccurlyeq", "≼", Rel),
    ("preceq", "⪯", Rel),
    ("prime", "′", Ord),
    ("prod", "∏", Op),
    ("propto", "∝", Rel),
    ("qed", "∎", Ord),
    ("rVert", "‖", Close),
    ("rangle", "⟩", Close),
    ("rbrace", "}", Close),
    ("rbrack", "]", Close),
    ("rceil", "⌉", Close),
    ("rfloor", "⌋", Close),
    ("rhd", "⊳", Bin),
    ("rightarrow", "→", Rel),
    ("rightharpoonup", "⇀", Rel),
    ("rightleftarrows", "⇄", Rel),
    ("rightleftharpoons", "⇌", Rel),
    ("rightsquigarrow", "⇝", Rel),
    ("risingdotseq", "≓", Rel),
    ("rvert", "|", Close),
    ("searrow", "↘", Rel),
    ("setminus", "∖", Bin),
    ("sharp", "♯", Ord),
    ("sim", "∼", Rel),
    ("simeq", "≃", Rel),
    ("smile", "⌣", Rel),
    ("spadesuit", "♠", Ord),
    ("sqcap", "⊓", Bin),
    ("sqcup", "⊔", Bin),
    ("square", "□", Ord),
    ("star", "⋆", Bin),
    ("subset", "⊂", Rel),
    ("subseteq", "⊆", Rel),
    ("subsetneq", "⊊", Rel),
    ("succ", "≻", Rel),
    ("succcurlyeq", "≽", Rel),
    ("succeq", "⪰", Rel),
    ("sum", "∑", Op),
    ("supset", "⊃", Rel),
    ("supseteq", "⊇", Rel),
    ("supsetneq", "⊋", Rel),
    ("surd", "√", Ord),
    ("swarrow", "↙", Rel),
    ("therefore", "∴", Rel),
    ("thicksim", "∼", Rel),
    ("times", "×", Bin),
    ("to", "→", Rel),
    ("top", "⊤", Ord),
    ("triangle", "△", Ord),
    ("triangleleft", "◁", Bin),
    ("trianglelefteq", "⊴", Rel),
    ("triangleq", "≜", Rel),
    ("triangleright", "▷", Bin),
    ("trianglerighteq", "⊵", Rel),
    ("uparrow", "↑", Rel),
    ("updownarrow", "↕", Rel),
    ("uplus", "⊎", Bin),
    ("vDash", "⊨", Rel),
    ("varDelta", "Δ", Ord),
    ("varGamma", "Γ", Ord),
    ("varLambda", "Λ", Ord),
    ("varOmega", "Ω", Ord),
    ("varPhi", "Φ", Ord),
    ("varPi", "Π", Ord),
    ("varPsi", "Ψ", Ord),
    ("varSigma", "Σ", Ord),
    ("varTheta", "Θ", Ord),
    ("varUpsilon", "Υ", Ord),
    ("varXi", "Ξ", Ord),
    ("varnothing", "∅", Ord),
    ("vdash", "⊢", Rel),
    ("vdots", "⋮", Ord),
    ("vee", "∨", Bin),
    ("vert", "|", Ord),
    ("wedge", "∧", Bin),
    ("wp", "℘", Ord),
    ("{", "{", Open),
    ("|", "‖", Ord),
    ("}", "}", Close),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(tex: &str) -> Vec<String> {
        let rows = display(tex).unwrap_or_else(|| panic!("not typeset: {tex}"));
        rows.into_iter()
            .map(|r| r.text.replace(NBSP, "·"))
            .collect()
    }

    #[test]
    fn printable_is_ascii_in_order() {
        let want: String = (' '..='~').collect();
        assert_eq!(PRINTABLE, want);
    }

    #[test]
    fn tables_are_sorted() {
        let names: Vec<&str> = SYMBOLS.iter().map(|(n, ..)| *n).collect();
        assert!(names.is_sorted(), "SYMBOLS out of order");
        let greek: Vec<&str> = GREEK.iter().map(|(n, _)| *n).collect();
        assert!(greek.is_sorted(), "GREEK out of order");
    }

    #[test]
    fn symbols_scripts_and_alphabets() {
        for (tex, want) in [
            ("\\alpha \\le \\beta", "α ≤ β"),
            ("x \\in \\mathbb{R}^n", "x ∈ ℝⁿ"),
            ("e^{i\\pi}", "e^(iπ)"),
            ("x_1, x_{n-1}, x^{2k}", "x₁, xₙ₋₁, x²ᵏ"),
            ("\\mathcal{O}(n \\log n)", "𝒪(n log n)"),
            ("\\mathbf{v} \\cdot \\vec{u}", "𝐯 · u⃗"),
            ("\\sum_{k=1}^{n} k", "∑ₖ₌₁ⁿ k"),
            ("\\lim_{x \\to 0} f(x)", "lim_(x→0) f(x)"),
            ("f'(x) + f''", "f′(x) + f″"),
            ("\\text{if } x > 0", "if x > 0"),
            ("\\{ x \\mid x \\neq 0 \\}", "{x ∣ x ≠ 0}"),
            ("\\vec{u_1}", "u⃗₁"),
            ("A^T, A^{-1}, 90^\\circ", "Aᵀ, A⁻¹, 90°"),
            ("a \\not\\in B, a \\not= b", "a ∉ B, a ≠ b"),
            ("N^{\\frac{1}{4}}", "N¹⁄⁴"),
            // marks without a script letter make a script only of their own (OBS-019)
            (
                "x^*, x^{**}, A^\\dagger, f^{\\prime\\prime}",
                "x∗, x∗∗, A†, f′′",
            ),
            ("5^{10*13+4}, \\Sigma^{*m}", "5^(10∗13+4), Σ^(∗m)"),
            ("e^{i\\theta'}, x^{2\\circ}, x^{*\\prime}", "eⁱᶿ′, x²°, x∗′"),
            ("\\sum_{i,j}", "∑ᵢ,ⱼ"),
        ] {
            assert_eq!(inline(tex).as_deref(), Some(want), "{tex}");
        }
    }

    /// Spacing is TeX's, by the atoms' classes, not the source's.
    #[test]
    fn spacing_follows_the_atoms() {
        for (tex, want) in [
            ("x^2+y^2=z^2", "x² + y² = z²"),
            ("a   =   b", "a = b"),
            ("-x + (-1)", "-x + (-1)"),
            ("-\\sum_i x_i", "-∑ᵢ xᵢ"),
            ("\\sin x \\cos(y)", "sin x cos(y)"),
            ("f(x,y)", "f(x, y)"),
            ("a \\quad b \\ \\ c", "a ·b ·c"),
            ("\\max_{e \\in C} \\{w(e)\\}", "max_(e∈C) {w(e)}"),
            ("x_{c \\log n}", "x_(c log n)"),
        ] {
            let got = inline(tex).map(|s| s.replace(NBSP, "·"));
            assert_eq!(got.as_deref(), Some(want), "{tex}");
        }
    }

    #[test]
    fn fractions_and_roots_on_a_line() {
        for (tex, want) in [
            ("\\frac{1}{2}", "1/2"),
            ("\\frac{a+b}{2}", "(a + b)/2"),
            ("\\frac{n(n+1)}{2}", "n(n + 1)/2"),
            ("\\frac{x}{2a}", "x/(2a)"),
            (
                "\\frac{d}{dx}, \\frac{\\partial f}{\\partial x}",
                "d/dx, ∂f/∂x",
            ),
            // fractions side by side take parentheses as any term beside them does
            (
                "\\frac{d}{dx} \\frac{\\partial f}{\\partial x}",
                "(d/dx)(∂f/∂x)",
            ),
            ("\\frac{1}{f(x)}", "1/f(x)"),
            ("2\\frac{a}{b}", "2(a/b)"),
            ("\\frac{a}{b}x", "(a/b)x"),
            ("\\frac{a}{b}^2", "(a/b)²"),
            ("\\frac12", "1/2"),
            (
                "\\sqrt{2}, \\sqrt{x+1}, \\sqrt[3]{8}, \\sqrt[n]{x}",
                "√2, √(x + 1), ∛8, ⁿ√x",
            ),
            ("\\frac{1}{\\sqrt{1-x^2}}", "1/√(1 - x²)"),
            // a root over a product covers it all (OBS-018)
            ("\\sqrt{2\\pi}, \\sqrt{2}\\pi", "√(2π), √2π"),
            (
                "\\sqrt{npq}, \\sqrt{x^2}, \\sqrt{\\sqrt{x}}",
                "√(npq), √x², √√x",
            ),
            (
                "\\sqrt{(x-1)(x+1)}, \\sqrt[3]{(x-1)(x+1)^{2}}",
                "√((x - 1)(x + 1)), ∛((x - 1)(x + 1)²)",
            ),
            (
                "\\sqrt{(-1)(-1)} \\ne \\sqrt{-1}\\sqrt{-1}",
                "√((-1)(-1)) ≠ √(-1)√(-1)",
            ),
            (
                "\\sqrt{|V|}, \\sqrt{f(x)}, \\sqrt{2.5}",
                "√(|V|), √f(x), √2.5",
            ),
        ] {
            assert_eq!(inline(tex).as_deref(), Some(want), "{tex}");
        }
    }

    /// Prose or shell code between two `$`s stays as written.
    #[test]
    fn prose_is_not_typeset() {
        assert_eq!(inline("name its me"), None);
        assert_eq!(inline("b 的值。所以"), None);
        assert_eq!(display("\\frac{valor final}{inicial}"), None);
        // words in a command's argument are TeX's, as are letters beside a command
        assert_eq!(
            inline("\\text{if and only if }x").as_deref(),
            Some("if and only if x")
        );
        assert_eq!(inline("O(n \\log n)").as_deref(), Some("O(n log n)"));
        assert_eq!(inline("k_{true}やm").as_deref(), Some("kₜᵣᵤₑやm"));
    }

    /// What it doesn't know leaves the whole formula as TeX.
    #[test]
    fn unknown_tex_is_none() {
        for tex in [
            "\\RR^n",
            "\\binom{n}{k}",
            "a & b",
            "a \\\\ b",
            "\\frac{a}",
            "x^",
            "{a",
            "a}",
            "x_1_2",
            "\\begin{cases} a \\\\ b \\end{cases}",
            "",
            "\\",
        ] {
            assert_eq!(inline(tex), None, "{tex}");
        }
        assert_eq!(display("\\RR"), None);
        assert_eq!(display("\\begin{align}\\end{align}"), None);
    }

    #[test]
    fn inline_matrices_go_on_their_line() {
        assert_eq!(
            inline("\\begin{bmatrix} 1 & 2 \\\\ 3 & 4 \\end{bmatrix}").as_deref(),
            Some("[1 2; 3 4]")
        );
        assert_eq!(
            inline("$`\\sqrt{2}`$".trim_matches('$')).as_deref(),
            Some("√2")
        );
    }

    #[test]
    fn display_rows_align_on_their_ampersands() {
        let tex = "\\begin{aligned}\nf(x) &= x^2 + 2x + 1 \\\\\n  &= (x+1)^2\n\\end{aligned}";
        assert_eq!(rows(tex), ["f(x) = x² + 2x + 1", "···· = (x + 1)²"]);
        // where each row's TeX is
        let got = display(tex).unwrap();
        assert_eq!(&tex[got[0].tex.clone()], "f(x) &= x^2 + 2x + 1");
        assert_eq!(&tex[got[1].tex.clone()], "&= (x+1)^2");
        // rows without `&` are centered
        assert_eq!(rows("a+b \\\\ c"), ["a + b", "··c"]);
    }

    #[test]
    fn matrices_and_cases_are_drawn_tall() {
        assert_eq!(
            rows("A = \\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix}"),
            ["A = ⎛1··0⎞", "····⎝0··1⎠"]
        );
        assert_eq!(
            rows(
                "|x| = \\begin{cases} x & x \\ge 0 \\\\ -x & x < 0 \\\\ 0 & \\text{else} \\end{cases}"
            ),
            ["······⎧x···x ≥ 0", "|x| = ⎨-x··x < 0", "······⎩0···else"]
        );
        assert_eq!(
            rows("\\left[\\begin{array}{cc} a & b \\\\ c & d \\end{array}\\right]^T"),
            ["⎡a··b⎤ᵀ", "⎣c··d⎦"]
        );
        assert_eq!(rows("\\binom{n}{k}"), ["⎛n⎞", "⎝k⎠"]);
    }

    /// Whatever the bytes, no panic: every prefix and suffix of formulas that use it all.
    #[test]
    fn never_panics() {
        let samples = [
            "A = \\begin{pmatrix} 1 & \\frac{a}{b} \\\\ \\sqrt[3]{x} & \\left( y \\right)^2 \\end{pmatrix}",
            "\\begin{aligned} x &= \\text{é ü} \\\\[2pt] \\mathbb{Z}_n &\\to \\vec{v}' \\cr \\end{aligned}",
            "\\left\\{ \\begin{array}{l|r} \\alpha & \\not\\in \\\\ \\overset{def}{=} & \\binom{n}{k} \\end{array} \\right.",
        ];
        for s in samples {
            for (i, _) in s.char_indices() {
                let _ = (
                    inline(&s[..i]),
                    display(&s[..i]),
                    inline(&s[i..]),
                    display(&s[i..]),
                );
            }
        }
        // nesting past the limit
        let deep = "{".repeat(500) + &"}".repeat(500);
        assert_eq!(inline(&deep), None);
    }

    /// Commands nested through arguments without braces stop at the limit, on a stack as small
    /// as the loader thread's: 100,000 of them overflowed it before (PAR-011).
    #[test]
    fn braceless_nesting_stops_at_the_limit() {
        let chains = [
            "\\sqrt".repeat(100_000) + " x",
            "x^".to_owned() + &"\\hat".repeat(100_000) + " y",
            "\\not".repeat(100_000) + "=",
            "\\frac".repeat(100_000) + "12",
            "\\mathbb".repeat(100_000) + " R",
        ];
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                for s in &chains {
                    assert_eq!(inline(s), None, "{}…", &s[..12]);
                    assert_eq!(display(s), None, "{}…", &s[..12]);
                }
            })
            .unwrap()
            .join()
            .unwrap();
        // a chain within the limit still typesets
        assert_eq!(inline("\\sqrt\\sqrt x").as_deref(), Some("√√x"));
    }
}
