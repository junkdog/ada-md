//! The generated documents, one construct each (BEN-009; ada-throughput in the vault). Each is about
//! [`SIZE`] bytes from a fixed seed, so every run and every commit sees the same bytes, and comes
//! with a check that the parsed `Doc` holds what was generated. A change to a generator's output
//! bumps its version: the version starts a new series in the results.

use std::collections::BTreeMap;
use std::fmt::Write;

use ada::doc::{Block, BlockKind, Doc, InlineStyle, Inlines, LinkKind, RunKind};

/// About 4 MB: past every cache but L3, and long enough that a sample isn't a handful of calls.
pub const SIZE: usize = 4_000_000;

/// SplitMix64 (Steele, Lea and Flood, 2014): well spread in every bit, which xorshift's low bits
/// aren't, for drawing small ranges.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// `lo..=hi`, by multiplying rather than by modulo (Lemire).
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        let n = (hi - lo + 1) as u128;
        lo + ((self.next() as u128 * n) >> 64) as usize
    }

    /// True one time in `n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.range(1, n) == 1
    }

    pub fn word(&mut self) -> &'static str {
        WORDS[self.range(0, WORDS.len() - 1)]
    }
}

/// Plain words, none with a character markdown gives a meaning to.
const WORDS: [&str; 64] = [
    "a",
    "an",
    "the",
    "of",
    "to",
    "in",
    "and",
    "or",
    "is",
    "it",
    "on",
    "by",
    "as",
    "at",
    "be",
    "we",
    "layout",
    "table",
    "parser",
    "column",
    "terminal",
    "viewer",
    "width",
    "row",
    "line",
    "block",
    "inline",
    "theme",
    "style",
    "glyph",
    "cell",
    "frame",
    "screen",
    "scroll",
    "search",
    "anchor",
    "heading",
    "paragraph",
    "markdown",
    "document",
    "measure",
    "wrap",
    "reload",
    "highlight",
    "language",
    "grammar",
    "token",
    "span",
    "buffer",
    "output",
    "input",
    "byte",
    "throughput",
    "ceiling",
    "kernel",
    "counter",
    "cycle",
    "instruction",
    "benchmark",
    "median",
    "sample",
    "commit",
    "series",
    "result",
];

/// A generated document and what its parse must hold.
pub struct Generated {
    pub version: u32,
    pub text: String,
    check: Expect,
}

enum Expect {
    /// Only paragraphs, this many, and no links.
    Paragraphs(usize),
    /// This many tables, each with its column count, in order.
    Tables(Vec<usize>),
    /// Links by kind: inline, reference, autolink, email, wiki, aliased wiki.
    Links([usize; 6]),
    /// These counts of [`census`]'s, by name; the names left out aren't checked.
    Census(Vec<(&'static str, usize)>),
}

impl Generated {
    /// What the parse holds against what was generated: `Err` names the first difference.
    pub fn check(&self, doc: &Doc) -> Result<(), String> {
        match &self.check {
            &Expect::Paragraphs(n) => {
                let got = doc
                    .blocks
                    .iter()
                    .filter(|b| matches!(b.kind, BlockKind::Paragraph(_)))
                    .count();
                if got != n || doc.blocks.len() != n {
                    return Err(format!(
                        "{n} paragraphs, got {got} of {} blocks",
                        doc.blocks.len()
                    ));
                }
                if !doc.links.is_empty() {
                    return Err(format!("no links, got {}", doc.links.len()));
                }
            }
            Expect::Tables(columns) => {
                let got: Vec<usize> = doc
                    .blocks
                    .iter()
                    .filter_map(|b| match &b.kind {
                        BlockKind::Table(t) => Some(t.column_count()),
                        _ => None,
                    })
                    .collect();
                if &got != columns {
                    return Err(format!("{} tables, got {}", columns.len(), got.len()));
                }
            }
            Expect::Links(want) => {
                let mut got = [0; 6];
                for link in &doc.links {
                    let i = match link.kind {
                        LinkKind::Inline => 0,
                        LinkKind::Reference => 1,
                        LinkKind::Autolink => 2,
                        LinkKind::Email => 3,
                        LinkKind::Wiki { aliased: false } => 4,
                        LinkKind::Wiki { aliased: true } => 5,
                        other => return Err(format!("an unexpected {other:?} link")),
                    };
                    got[i] += 1;
                }
                if &got != want {
                    return Err(format!("links by kind {want:?}, got {got:?}"));
                }
            }
            Expect::Census(want) => {
                let got = census(doc);
                for &(key, n) in want {
                    let g = got.get(key).copied().unwrap_or(0);
                    if g != n {
                        return Err(format!("{key}: {n}, got {g}"));
                    }
                }
            }
        }
        Ok(())
    }
}

/// The generators by id.
pub fn by_id(id: &str) -> Option<Generated> {
    match id {
        "prose" => Some(prose()),
        "tables" => Some(tables()),
        "links" => Some(links()),
        "unicode" => Some(unicode()),
        "emphasis" => Some(emphasis()),
        "code-span" => Some(code_spans()),
        "footnotes" => Some(footnotes()),
        "headings" => Some(headings()),
        "lists" => Some(lists()),
        "quotes" => Some(quotes()),
        "table-wide" => Some(table_wide()),
        "fences" => Some(fences()),
        "html" => Some(html()),
        "long-lines" => Some(long_lines()),
        _ => None,
    }
}

/// Words into `out` as a sentence, soft-wrapped at about 80 columns as notes are; `col` is the
/// column the line is at.
fn sentence(rng: &mut Rng, out: &mut String, col: &mut usize) {
    let words = rng.range(6, 18);
    for i in 0..words {
        let word = rng.word();
        if *col > 0 && *col + 1 + word.len() > 80 {
            out.push('\n');
            *col = 0;
        } else if *col > 0 {
            out.push(' ');
            *col += 1;
        }
        if i == 0 {
            let mut chars = word.chars();
            out.extend(chars.next().map(|c| c.to_ascii_uppercase()));
            out.push_str(chars.as_str());
        } else {
            out.push_str(word);
        }
        *col += word.len();
    }
    out.push('.');
    *col += 1;
}

/// Paragraphs of ASCII words, 3 to 8 sentences each: the best case, which every other row is
/// read against.
fn prose() -> Generated {
    let mut rng = Rng::new(0x5052_4f53);
    let mut text = String::with_capacity(SIZE + 1024);
    let mut paragraphs = 0;
    while text.len() < SIZE {
        let mut col = 0;
        for _ in 0..rng.range(3, 8) {
            sentence(&mut rng, &mut text, &mut col);
        }
        text.push_str("\n\n");
        paragraphs += 1;
    }
    Generated {
        version: 1,
        text,
        check: Expect::Paragraphs(paragraphs),
    }
}

/// Tables of 4 to 8 columns and 10 to 40 rows, cells of 1 to 4 words, now and then bold or code,
/// each after a short paragraph.
fn tables() -> Generated {
    let mut rng = Rng::new(0x5441_424c);
    let mut text = String::with_capacity(SIZE + 64 * 1024);
    let mut columns = Vec::new();
    let cell = |rng: &mut Rng, text: &mut String| {
        let words = rng.range(1, 4);
        let mark = match rng.range(1, 8) {
            1 => "**",
            2 => "`",
            _ => "",
        };
        text.push_str(mark);
        for i in 0..words {
            if i > 0 {
                text.push(' ');
            }
            text.push_str(rng.word());
        }
        text.push_str(mark);
    };
    while text.len() < SIZE {
        let mut col = 0;
        sentence(&mut rng, &mut text, &mut col);
        text.push_str("\n\n");
        let cols = rng.range(4, 8);
        for row in 0..rng.range(10, 40) + 2 {
            text.push('|');
            for _ in 0..cols {
                text.push(' ');
                if row == 1 {
                    text.push_str(["---", ":--", "--:", ":-:"][rng.range(0, 3)]);
                } else {
                    cell(&mut rng, &mut text);
                }
                text.push_str(" |");
            }
            text.push('\n');
        }
        text.push('\n');
        columns.push(cols);
    }
    Generated {
        version: 1,
        text,
        check: Expect::Tables(columns),
    }
}

/// Paragraphs where about one word in five is a link: inline, reference (defined after the
/// paragraph, a label each), autolink, email, wikilink and aliased wikilink.
fn links() -> Generated {
    let mut rng = Rng::new(0x4c49_4e4b);
    let mut text = String::with_capacity(SIZE + 1024);
    let mut kinds = [0; 6];
    let mut label = 0;
    while text.len() < SIZE {
        let mut defs = String::new();
        let mut col = 0;
        for _ in 0..rng.range(30, 80) {
            if col > 72 {
                text.push('\n');
                col = 0;
            } else if col > 0 {
                text.push(' ');
                col += 1;
            }
            let start = text.len();
            if rng.one_in(5) {
                let (a, b) = (rng.word(), rng.word());
                let n = rng.range(0, 9999);
                let kind = rng.range(0, 5);
                match kind {
                    0 => write!(text, "[{a} {b}](https://example.com/{a}/{n})"),
                    1 => {
                        label += 1;
                        writeln!(defs, "[r{label}]: https://example.org/{b}/{n}").unwrap();
                        write!(text, "[{a} {b}][r{label}]")
                    }
                    2 => write!(text, "<https://example.net/{a}/{n}>"),
                    3 => write!(text, "<{a}{n}@example.com>"),
                    4 => write!(text, "[[{a} {b} {n}]]"),
                    _ => write!(text, "[[{a} {n}|{b}]]"),
                }
                .unwrap();
                kinds[kind] += 1;
            } else {
                text.push_str(rng.word());
            }
            col += text.len() - start;
        }
        text.push_str(".\n\n");
        if !defs.is_empty() {
            text.push_str(&defs);
            text.push('\n');
        }
    }
    Generated {
        version: 1,
        text,
        check: Expect::Links(kinds),
    }
}

/// What a parse holds, counted by name: blocks by kind, wherever they're nested (`h1` to `h6`
/// by level, `item` and `task` per list item, `callout` per quote with one), the bytes of text
/// under each inline style (`em-bytes`, `strong-bytes`, `strike-bytes`, `code-span-bytes`) and
/// of code blocks (`code-bytes`), hard breaks, footnote references, and the doc's footnotes,
/// links and anchors.
pub fn census(doc: &Doc) -> BTreeMap<&'static str, usize> {
    const LEVELS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];
    const STYLES: [(InlineStyle, &str); 4] = [
        (InlineStyle::EMPH, "em-bytes"),
        (InlineStyle::STRONG, "strong-bytes"),
        (InlineStyle::STRIKE, "strike-bytes"),
        (InlineStyle::CODE, "code-span-bytes"),
    ];
    type Counts = BTreeMap<&'static str, usize>;
    fn add(n: &mut Counts, key: &'static str, by: usize) {
        *n.entry(key).or_default() += by;
    }
    fn inlines(i: &Inlines, n: &mut Counts) {
        for run in &i.runs {
            match run.kind {
                RunKind::Text => {
                    for (style, key) in STYLES {
                        if run.style.contains(style) {
                            add(n, key, run.text.len());
                        }
                    }
                }
                RunKind::HardBreak => add(n, "hardbreak", 1),
                RunKind::FootnoteRef(_) => add(n, "footnote-ref", 1),
                _ => {}
            }
        }
    }
    fn walk(blocks: &[Block], n: &mut Counts) {
        for block in blocks {
            match &block.kind {
                BlockKind::Heading {
                    level, inlines: i, ..
                } => {
                    add(n, "heading", 1);
                    add(n, LEVELS[usize::from(*level) - 1], 1);
                    inlines(i, n);
                }
                BlockKind::Paragraph(i) => {
                    add(n, "paragraph", 1);
                    inlines(i, n);
                }
                BlockKind::List(list) => {
                    add(n, "list", 1);
                    for item in &list.items {
                        add(n, "item", 1);
                        add(n, "task", usize::from(item.task.is_some()));
                        walk(&item.blocks, n);
                    }
                }
                BlockKind::Quote { callout, blocks } => {
                    add(n, "quote", 1);
                    add(n, "callout", usize::from(callout.is_some()));
                    walk(blocks, n);
                }
                BlockKind::Code(code) => {
                    add(n, "code", 1);
                    add(n, "code-bytes", code.text.len());
                }
                BlockKind::Table(t) => {
                    add(n, "table", 1);
                    for row in std::iter::once(&t.header).chain(&t.rows) {
                        row.cells.iter().for_each(|c| inlines(c, n));
                    }
                }
                BlockKind::Image { .. } => add(n, "image", 1),
                BlockKind::Rule => add(n, "rule", 1),
                BlockKind::Html(_) => add(n, "html", 1),
                BlockKind::FootnoteDefs => add(n, "footnote-section", 1),
            }
        }
    }
    let mut n = Counts::new();
    walk(&doc.blocks, &mut n);
    for note in &doc.footnotes {
        walk(&note.blocks, &mut n);
    }
    add(&mut n, "footnote", doc.footnotes.len());
    add(&mut n, "link", doc.links.len());
    add(&mut n, "anchor", doc.anchors.len());
    n
}

/// A token into `out` after a space, or on a new line starting with `prefix` once the line's
/// text passes 80 columns; `col` is the column of the line's text, after its prefix.
fn put(out: &mut String, col: &mut usize, prefix: &str, token: &str) {
    if *col > 0 && *col + 1 + token.len() > 80 {
        out.push('\n');
        out.push_str(prefix);
        *col = 0;
    } else if *col > 0 {
        out.push(' ');
        *col += 1;
    }
    out.push_str(token);
    *col += token.len();
}

/// `n` sentences through [`put`] from where `out` is, each new line starting with `prefix`,
/// then a line end.
fn sentences(rng: &mut Rng, out: &mut String, prefix: &str, n: usize) {
    let mut col = 0;
    let mut token = String::new();
    for _ in 0..n {
        let words = rng.range(6, 18);
        for i in 0..words {
            token.clear();
            token.push_str(rng.word());
            if i == 0 {
                token[..1].make_ascii_uppercase();
            }
            if i == words - 1 {
                token.push('.');
            }
            put(out, &mut col, prefix, &token);
        }
    }
    out.push('\n');
}

/// A paragraph of one to `max` sentences, every line starting with `prefix`.
fn paragraph(rng: &mut Rng, out: &mut String, prefix: &str, max: usize) {
    out.push_str(prefix);
    let n = rng.range(1, max);
    sentences(rng, out, prefix, n);
}

/// Words outside ASCII: CJK, Hangul, accents precomposed and combining, Cyrillic, Greek,
/// Devanagari and Thai (combining vowel signs), Hebrew, and emoji with skin tones, variation
/// selectors, flags and ZWJ sequences.
const UNICODE: [&str; 32] = [
    "表格",
    "終端機",
    "解析器",
    "幅",
    "日本語",
    "ひらがな",
    "カタカナ",
    "한국어",
    "터미널",
    "中文字幕",
    "café",
    "naïve",
    "Ångström",
    "façade",
    "e\u{301}cole",
    "man\u{303}ana",
    "u\u{308}ber",
    "a\u{30a}\u{301}",
    "терминал",
    "ширина",
    "πλάτος",
    "हिन्दी",
    "ภาษาไทย",
    "עברית",
    "🦀",
    "👍🏽",
    "👩‍💻",
    "👨‍👩‍👧‍👦",
    "🏳️‍🌈",
    "🇸🇪",
    "⚠️",
    "❤️",
];

/// Paragraphs as prose's, where about one word in three is from [`UNICODE`].
fn unicode() -> Generated {
    let mut rng = Rng::new(0x554e_4943);
    let mut text = String::with_capacity(SIZE + 1024);
    let mut paragraphs = 0;
    while text.len() < SIZE {
        let mut col = 0;
        for _ in 0..rng.range(30, 120) {
            let word = if rng.one_in(3) {
                UNICODE[rng.range(0, UNICODE.len() - 1)]
            } else {
                rng.word()
            };
            put(&mut text, &mut col, "", word);
        }
        text.push_str(".\n\n");
        paragraphs += 1;
    }
    Generated {
        version: 1,
        text,
        check: Expect::Paragraphs(paragraphs),
    }
}

/// Paragraphs where about one token in three is emphasized: `*em*` and `_em_`, `**strong**`
/// and `__strong__`, `~~strike~~`, `***both***`, and emphasis nested in strong.
fn emphasis() -> Generated {
    let mut rng = Rng::new(0x454d_5048);
    let mut text = String::with_capacity(SIZE + 1024);
    let (mut em, mut strong, mut strike, mut paragraphs) = (0, 0, 0, 0);
    let mut token = String::new();
    while text.len() < SIZE {
        let mut col = 0;
        for _ in 0..rng.range(30, 90) {
            token.clear();
            if rng.one_in(3) {
                let (a, b) = (rng.word(), rng.word());
                let both = a.len() + 1 + b.len();
                match rng.range(0, 6) {
                    0 => {
                        write!(token, "*{a} {b}*").unwrap();
                        em += both;
                    }
                    1 => {
                        write!(token, "_{a}_").unwrap();
                        em += a.len();
                    }
                    2 => {
                        write!(token, "**{a} {b}**").unwrap();
                        strong += both;
                    }
                    3 => {
                        write!(token, "__{a}__").unwrap();
                        strong += a.len();
                    }
                    4 => {
                        write!(token, "~~{a} {b}~~").unwrap();
                        strike += both;
                    }
                    5 => {
                        write!(token, "***{a}***").unwrap();
                        em += a.len();
                        strong += a.len();
                    }
                    _ => {
                        let c = rng.word();
                        write!(token, "**{a} *{b}* {c}**").unwrap();
                        strong += both + 1 + c.len();
                        em += b.len();
                    }
                }
            } else {
                token.push_str(rng.word());
            }
            put(&mut text, &mut col, "", &token);
        }
        text.push_str(".\n\n");
        paragraphs += 1;
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("paragraph", paragraphs),
            ("em-bytes", em),
            ("strong-bytes", strong),
            ("strike-bytes", strike),
            ("link", 0),
        ]),
    }
}

/// Paragraphs where about one token in two is a code span: a word, a path, a call, or a
/// double-backtick span with a backtick inside.
fn code_spans() -> Generated {
    let mut rng = Rng::new(0x434f_4445);
    let mut text = String::with_capacity(SIZE + 1024);
    let (mut code, mut paragraphs) = (0, 0);
    let mut token = String::new();
    while text.len() < SIZE {
        let mut col = 0;
        for _ in 0..rng.range(30, 90) {
            token.clear();
            if rng.one_in(2) {
                let (a, b) = (rng.word(), rng.word());
                let inner = match rng.range(0, 3) {
                    0 => {
                        write!(token, "`{a}`").unwrap();
                        a.len()
                    }
                    1 => {
                        write!(token, "`{a}::{b}`").unwrap();
                        a.len() + 2 + b.len()
                    }
                    2 => {
                        let n = rng.range(0, 999);
                        let call = format!("{a}({b}, {n})");
                        write!(token, "`{call}`").unwrap();
                        call.len()
                    }
                    // the spaces inside the delimiters are stripped
                    _ => {
                        write!(token, "`` {a}`{b} ``").unwrap();
                        a.len() + 1 + b.len()
                    }
                };
                code += inner;
            } else {
                token.push_str(rng.word());
            }
            put(&mut text, &mut col, "", &token);
        }
        text.push_str(".\n\n");
        paragraphs += 1;
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("paragraph", paragraphs),
            ("code-span-bytes", code),
            ("link", 0),
        ]),
    }
}

/// Paragraphs where about one word in eight carries a footnote reference, one in ten of them
/// to an earlier note; each paragraph's new notes are defined after it in reverse order, so
/// the footnote section reorders them.
fn footnotes() -> Generated {
    let mut rng = Rng::new(0x464f_4f54);
    let mut text = String::with_capacity(SIZE + 1024);
    let (mut notes, mut refs, mut paragraphs) = (0, 0, 0);
    let mut token = String::new();
    while text.len() < SIZE {
        let mut fresh = Vec::new();
        let mut col = 0;
        for _ in 0..rng.range(30, 90) {
            token.clear();
            token.push_str(rng.word());
            if rng.one_in(8) {
                let label = if notes > 0 && rng.one_in(10) {
                    rng.range(1, notes)
                } else {
                    notes += 1;
                    fresh.push(notes);
                    notes
                };
                write!(token, "[^n{label}]").unwrap();
                refs += 1;
            }
            put(&mut text, &mut col, "", &token);
        }
        text.push_str(".\n\n");
        paragraphs += 1;
        for label in fresh.into_iter().rev() {
            write!(text, "[^n{label}]: ").unwrap();
            let n = rng.range(1, 2);
            sentences(&mut rng, &mut text, "    ", n);
            text.push('\n');
            paragraphs += 1;
        }
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("paragraph", paragraphs),
            ("footnote", notes),
            ("footnote-ref", refs),
            ("footnote-section", 1),
            ("link", refs),
        ]),
    }
}

/// Short sections, a heading of any level and a paragraph or two: ATX headings, some closed
/// (`## a ##`), and setext ones for levels 1 and 2.
fn headings() -> Generated {
    let mut rng = Rng::new(0x4845_4144);
    let mut text = String::with_capacity(SIZE + 1024);
    let (mut levels, mut paragraphs) = ([0; 6], 0);
    while text.len() < SIZE {
        let level = rng.range(1, 6);
        let mut title = String::new();
        for i in 0..rng.range(2, 6) {
            if i > 0 {
                title.push(' ');
            }
            title.push_str(rng.word());
        }
        let hashes = "#".repeat(level);
        if level <= 2 && rng.one_in(4) {
            let underline = if level == 1 { "=" } else { "-" };
            writeln!(text, "{title}\n{}", underline.repeat(title.len())).unwrap();
        } else if rng.one_in(4) {
            writeln!(text, "{hashes} {title} {hashes}").unwrap();
        } else {
            writeln!(text, "{hashes} {title}").unwrap();
        }
        text.push('\n');
        levels[level - 1] += 1;
        for _ in 0..rng.range(1, 2) {
            paragraph(&mut rng, &mut text, "", 3);
            text.push('\n');
            paragraphs += 1;
        }
    }
    let headings = levels.iter().sum();
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("heading", headings),
            ("h1", levels[0]),
            ("h2", levels[1]),
            ("h3", levels[2]),
            ("h4", levels[3]),
            ("h5", levels[4]),
            ("h6", levels[5]),
            ("anchor", headings),
            ("paragraph", paragraphs),
        ]),
    }
}

/// Lists and the counts [`lists`] checks: lists, items, tasks.
#[derive(Default)]
struct ListCounts {
    lists: usize,
    items: usize,
    tasks: usize,
}

/// A tight list at `indent`, two to six items of a sentence or two, about one item in three
/// with a list nested in it, four levels at most. Ordered lists nested in an item start at 1,
/// as only those interrupt the item's text; bullet lists are task lists one time in three.
fn list(rng: &mut Rng, text: &mut String, indent: usize, depth: usize, n: &mut ListCounts) {
    let ordered = rng.one_in(3);
    let start = if ordered && depth == 0 {
        rng.range(1, 20)
    } else {
        1
    };
    let bullet = ["-", "*", "+"][rng.range(0, 2)];
    let tasks = !ordered && rng.one_in(3);
    n.lists += 1;
    for i in 0..rng.range(2, 6) {
        let marker = if ordered {
            format!("{}.", start + i)
        } else {
            bullet.to_owned()
        };
        let inner = " ".repeat(indent + marker.len() + 1);
        write!(text, "{}{marker} ", &inner[..indent]).unwrap();
        if tasks {
            text.push_str(["[ ] ", "[x] "][rng.range(0, 1)]);
            n.tasks += 1;
        }
        let mut col = 0;
        let mut token = String::new();
        for w in 0..rng.range(4, 24) {
            token.clear();
            token.push_str(rng.word());
            if w == 0 {
                token[..1].make_ascii_uppercase();
            }
            put(text, &mut col, &inner, &token);
        }
        text.push_str(".\n");
        n.items += 1;
        if depth < 3 && rng.one_in(3) {
            list(rng, text, inner.len(), depth + 1, n);
        }
    }
}

/// Nested bullet, ordered and task lists ([`list`]), a paragraph between each two.
fn lists() -> Generated {
    let mut rng = Rng::new(0x4c49_5354);
    let mut text = String::with_capacity(SIZE + 1024);
    let mut n = ListCounts::default();
    let mut paragraphs = 0;
    while text.len() < SIZE {
        paragraph(&mut rng, &mut text, "", 1);
        text.push('\n');
        paragraphs += 1;
        list(&mut rng, &mut text, 0, 0, &mut n);
        text.push('\n');
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("list", n.lists),
            ("item", n.items),
            ("task", n.tasks),
            // a tight item's text is a paragraph of its own
            ("paragraph", paragraphs + n.items),
        ]),
    }
}

/// A quote of a paragraph or two at `prefix`, with a quote nested in it about one time in
/// two, four levels at most; the top level's a GitHub callout one time in three.
fn quote(rng: &mut Rng, text: &mut String, prefix: &str, n: &mut [usize; 3]) {
    const CALLOUTS: [&str; 5] = ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"];
    let blank = prefix.trim_end();
    if prefix == "> " && rng.one_in(3) {
        writeln!(text, "> [!{}]", CALLOUTS[rng.range(0, 4)]).unwrap();
        n[1] += 1;
    }
    n[0] += 1;
    for p in 0..rng.range(1, 2) {
        if p > 0 {
            writeln!(text, "{blank}").unwrap();
        }
        paragraph(rng, text, prefix, 3);
        n[2] += 1;
    }
    if prefix.len() < 8 && rng.one_in(2) {
        writeln!(text, "{blank}").unwrap();
        quote(rng, text, &format!("{prefix}> "), n);
    }
}

/// Nested quotes and GitHub callouts ([`quote`]), a paragraph between each two.
fn quotes() -> Generated {
    let mut rng = Rng::new(0x5155_4f54);
    let mut text = String::with_capacity(SIZE + 1024);
    // quotes, callouts, paragraphs
    let mut n = [0; 3];
    while text.len() < SIZE {
        paragraph(&mut rng, &mut text, "", 1);
        text.push('\n');
        n[2] += 1;
        quote(&mut rng, &mut text, "> ", &mut n);
        text.push('\n');
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("quote", n[0]),
            ("callout", n[1]),
            ("paragraph", n[2]),
        ]),
    }
}

/// Tables of 20 to 30 columns and 10 to 30 rows, cells of 3 to 12 words, now and then bold or
/// code, each after a short paragraph. Laid out as a grid, not record cards
/// (`TableMode::Grid`), so the column distribution is under pressure.
fn table_wide() -> Generated {
    let mut rng = Rng::new(0x5749_4445);
    let mut text = String::with_capacity(SIZE + 64 * 1024);
    let mut columns = Vec::new();
    while text.len() < SIZE {
        paragraph(&mut rng, &mut text, "", 1);
        text.push('\n');
        let cols = rng.range(20, 30);
        for row in 0..rng.range(10, 30) + 2 {
            text.push('|');
            for _ in 0..cols {
                text.push(' ');
                if row == 1 {
                    text.push_str(["---", ":--", "--:", ":-:"][rng.range(0, 3)]);
                } else {
                    let mark = match rng.range(1, 8) {
                        1 => "**",
                        2 => "`",
                        _ => "",
                    };
                    text.push_str(mark);
                    for i in 0..rng.range(3, 12) {
                        if i > 0 {
                            text.push(' ');
                        }
                        text.push_str(rng.word());
                    }
                    text.push_str(mark);
                }
                text.push_str(" |");
            }
            text.push('\n');
        }
        text.push('\n');
        columns.push(cols);
    }
    Generated {
        version: 1,
        text,
        check: Expect::Tables(columns),
    }
}

/// A line of code-like text: statements, calls, comments, blank lines, and argument lists long
/// enough to wrap at 100 columns; indented by spaces, or a tab one time in eight.
fn code_line(rng: &mut Rng, line: &mut String) {
    line.clear();
    if rng.one_in(8) {
        line.push('\t');
    } else {
        line.push_str(&"    "[..rng.range(0, 1) * 4]);
        line.push_str(&"        "[..rng.range(0, 2) * 4]);
    }
    let (a, b, c) = (rng.word(), rng.word(), rng.word());
    let n = rng.range(0, 9999);
    match rng.range(0, 5) {
        0 => write!(line, "let {a}_{b} = {c}({b}, {n});").unwrap(),
        1 => write!(line, "{a}.{b}({c}_{n});").unwrap(),
        2 => write!(line, "if {a} > {n} {{ return {b}; }}").unwrap(),
        3 => {
            line.push_str("//");
            for _ in 0..rng.range(3, 10) {
                line.push(' ');
                line.push_str(rng.word());
            }
        }
        4 => {
            write!(line, "{a}({b}").unwrap();
            for _ in 0..rng.range(10, 30) {
                write!(line, ", {}", rng.word()).unwrap();
            }
            line.push_str(");");
        }
        _ => line.clear(),
    }
}

/// Fenced blocks of `txt`, 5 to 40 lines ([`code_line`]), fenced by backticks or now and then
/// tildes, each after a short paragraph.
fn fences() -> Generated {
    let mut rng = Rng::new(0x4645_4e43);
    let mut text = String::with_capacity(SIZE + 1024);
    let (mut blocks, mut bytes, mut paragraphs) = (0, 0, 0);
    let mut line = String::new();
    while text.len() < SIZE {
        paragraph(&mut rng, &mut text, "", 2);
        text.push('\n');
        paragraphs += 1;
        let fence = if rng.one_in(4) { "~~~" } else { "```" };
        writeln!(text, "{fence}txt").unwrap();
        for _ in 0..rng.range(5, 40) {
            code_line(&mut rng, &mut line);
            writeln!(text, "{line}").unwrap();
            bytes += line.len() + 1;
        }
        writeln!(text, "{fence}\n").unwrap();
        blocks += 1;
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("code", blocks),
            ("code-bytes", bytes),
            ("paragraph", paragraphs),
        ]),
    }
}

/// HTML blocks as READMEs have them (a `div` of paragraphs, a table, a centered image,
/// `details` around markdown, comments), between paragraphs where about one token in six is an
/// inline tag: `kbd`, `span`, `b`, `sub`, `sup`, `a`, a comment, or `<br>`, a hard break.
fn html() -> Generated {
    let mut rng = Rng::new(0x4854_4d4c);
    let mut text = String::with_capacity(SIZE + 1024);
    let (mut blocks, mut breaks, mut paragraphs) = (0, 0, 0);
    let mut token = String::new();
    while text.len() < SIZE {
        let mut col = 0;
        for _ in 0..rng.range(30, 90) {
            token.clear();
            let (a, b) = (rng.word(), rng.word());
            if rng.one_in(6) {
                match rng.range(0, 8) {
                    0 => write!(token, "<kbd>{a}</kbd>").unwrap(),
                    1 => write!(token, "<span class=\"{a}\">{a} {b}</span>").unwrap(),
                    2 => write!(token, "<b>{a}</b>").unwrap(),
                    3 => write!(token, "<sub>{}</sub>", rng.range(0, 99)).unwrap(),
                    4 => write!(token, "<sup>{}</sup>", rng.range(0, 99)).unwrap(),
                    5 => write!(token, "<a href=\"https://example.com/{a}\">{b}</a>").unwrap(),
                    // a word first: a line starting `<!--` would open an HTML block
                    6 => write!(token, "{a} <!-- {b} -->").unwrap(),
                    _ => {
                        token.push_str(["<br>", "<br/>", "<br />"][rng.range(0, 2)]);
                        breaks += 1;
                    }
                }
            } else {
                token.push_str(a);
            }
            put(&mut text, &mut col, "", &token);
        }
        text.push_str(".\n\n");
        paragraphs += 1;
        let (a, b) = (rng.word(), rng.word());
        match rng.range(0, 4) {
            0 => {
                writeln!(text, "<div class=\"{a}\">").unwrap();
                for _ in 0..rng.range(1, 3) {
                    text.push_str("<p>");
                    paragraph(&mut rng, &mut text, "", 1);
                    text.pop();
                    text.push_str("</p>\n");
                }
                text.push_str("</div>\n\n");
                blocks += 1;
            }
            1 => {
                text.push_str("<table>\n");
                for _ in 0..rng.range(2, 8) {
                    text.push_str("  <tr>");
                    for _ in 0..rng.range(2, 5) {
                        write!(text, "<td>{}</td>", rng.word()).unwrap();
                    }
                    text.push_str("</tr>\n");
                }
                text.push_str("</table>\n\n");
                blocks += 1;
            }
            2 => {
                writeln!(
                    text,
                    "<p align=\"center\">\n  <img src=\"{a}.png\" alt=\"{b}\">\n</p>\n"
                )
                .unwrap();
                blocks += 1;
            }
            3 => {
                writeln!(text, "<details>\n<summary>{a} {b}</summary>\n").unwrap();
                paragraph(&mut rng, &mut text, "", 3);
                text.push_str("\n</details>\n\n");
                blocks += 2;
                paragraphs += 1;
            }
            // a comment alone is no block of ada's
            _ => writeln!(text, "<!-- {a} {b} -->\n").unwrap(),
        }
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("html", blocks),
            ("hardbreak", breaks),
            ("paragraph", paragraphs),
            ("link", 0),
        ]),
    }
}

/// Paragraphs of one line of 2 to 16 KB, and one time in three, of two to six lines of 200
/// bytes to 3 KB joined by hard breaks (two spaces or a backslash).
fn long_lines() -> Generated {
    let mut rng = Rng::new(0x4c4f_4e47);
    let mut text = String::with_capacity(SIZE + 32 * 1024);
    let (mut breaks, mut paragraphs) = (0, 0);
    let line = |rng: &mut Rng, text: &mut String, bytes: usize| {
        let start = text.len();
        text.push_str(rng.word());
        while text.len() - start < bytes {
            text.push(' ');
            text.push_str(rng.word());
        }
    };
    while text.len() < SIZE {
        if rng.one_in(3) {
            let lines = rng.range(2, 6);
            for i in 0..lines {
                let bytes = rng.range(200, 3000);
                line(&mut rng, &mut text, bytes);
                if i + 1 < lines {
                    text.push_str(if rng.one_in(2) { "  \n" } else { "\\\n" });
                    breaks += 1;
                }
            }
        } else {
            let bytes = rng.range(2000, 16000);
            line(&mut rng, &mut text, bytes);
        }
        text.push_str(".\n\n");
        paragraphs += 1;
    }
    Generated {
        version: 1,
        text,
        check: Expect::Census(vec![
            ("paragraph", paragraphs),
            ("hardbreak", breaks),
            ("link", 0),
        ]),
    }
}

/// FNV-1a over the bytes: what the results record, so a changed output shows even when a
/// version wasn't bumped.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
