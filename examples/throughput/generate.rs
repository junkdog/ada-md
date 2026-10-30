//! The generated documents, one construct each (BEN-009; ada-throughput in the vault). Each is about
//! [`SIZE`] bytes from a fixed seed, so every run and every commit sees the same bytes, and comes
//! with a check that the parsed `Doc` holds what was generated. A change to a generator's output
//! bumps its version: the version starts a new series in the results.

use std::fmt::Write;

use ada::doc::{BlockKind, Doc, LinkKind};

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

/// FNV-1a over the bytes: what the results record, so a changed output shows even when a
/// version wasn't bumped.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
