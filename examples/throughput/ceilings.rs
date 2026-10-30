//! The ceiling kernels (BEN-009; ada-throughput in the vault, Ceilings): passes doing only the work no
//! implementation of a stage can skip, timed on the same input and core as ada's stage.

use std::hint::black_box;

use ada::doc::{BlockKind, Doc, Inlines};
use pulldown_cmark::{Options, Parser};
use unicode_width::UnicodeWidthStr;

/// C0: the input summed as `u64` words, once. Nothing that reads its input beats it.
pub fn read(bytes: &[u8]) -> u64 {
    let (words, tail) = bytes.as_chunks::<8>();
    let tail = tail.iter().map(|&b| u64::from(b)).sum::<u64>();
    words
        .iter()
        .map(|&w| u64::from_ne_bytes(w))
        .fold(tail, u64::wrapping_add)
}

/// C1 for print: the output's bytes written once, into a buffer allocated beforehand.
pub fn write(out: &[u8], into: &mut [u8]) {
    into.copy_from_slice(out);
    black_box(into);
}

/// A set of ASCII bytes a format gives a meaning to, for [`Set::classify`].
pub struct Set {
    lo: [u8; 16],
    hi: [u8; 16],
    table: [bool; 256],
}

/// Markdown's: line ends, and every character that opens, closes or marks a construct.
pub const MARKDOWN: &[u8] = b"\n!\"#&'()*+-.0123456789:<=>[\\]_`|~";
/// JSON's: structure, strings and numbers.
pub const JSON: &[u8] = b"\n\",-.0123456789:[\\]{}";
/// A log's, as `logs.rs` tokenizes: dates, numbers, brackets, quotes, keys and paths.
pub const LOG: &[u8] = b"\n\"'()-./0123456789:=[]|";

impl Set {
    /// The two nibble tables of the SIMD classifier (simdjson's): byte `b` is in the set when
    /// `lo[b & 15] & hi[b >> 4]` isn't zero. Exact for any ASCII set, as ASCII has eight high
    /// nibbles and the tables give each its own bit.
    pub fn new(bytes: &[u8]) -> Self {
        let mut set = Set {
            lo: [0; 16],
            hi: [0; 16],
            table: [false; 256],
        };
        for &b in bytes {
            assert!(b.is_ascii(), "the classifier takes ASCII sets");
            let bit = 1 << (b >> 4);
            set.hi[usize::from(b >> 4)] = bit;
            set.lo[usize::from(b & 15)] |= bit;
            set.table[usize::from(b)] = true;
        }
        set
    }

    /// C1 for parse and the lexers: every byte classified, every line end found. The counts of
    /// both, which a tokenizer's first pass has to know.
    pub fn classify(&self, bytes: &[u8]) -> (usize, usize) {
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            // SAFETY: the CPU has AVX2, checked just above.
            return unsafe { self.classify_avx2(bytes) };
        }
        self.classify_scalar(bytes)
    }

    fn classify_scalar(&self, bytes: &[u8]) -> (usize, usize) {
        let marks = bytes
            .iter()
            .filter(|&&b| self.table[usize::from(b)])
            .count();
        let lines = bytes.iter().filter(|&&b| b == b'\n').count();
        (marks, lines)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    fn classify_avx2(&self, bytes: &[u8]) -> (usize, usize) {
        use std::arch::x86_64::*;

        let (chunks, tail) = bytes.as_chunks::<32>();
        let (mut marks, mut lines) = self.classify_scalar(tail);
        // SAFETY: the loads read 16 and 32 bytes from arrays and chunks of that size.
        unsafe {
            let lo = _mm256_broadcastsi128_si256(_mm_loadu_si128(self.lo.as_ptr().cast()));
            let hi = _mm256_broadcastsi128_si256(_mm_loadu_si128(self.hi.as_ptr().cast()));
            let nibble = _mm256_set1_epi8(0x0f);
            let newline = _mm256_set1_epi8(b'\n' as i8);
            let zero = _mm256_setzero_si256();
            for chunk in chunks {
                let v = _mm256_loadu_si256(chunk.as_ptr().cast());
                let l = _mm256_shuffle_epi8(lo, _mm256_and_si256(v, nibble));
                let h = _mm256_shuffle_epi8(hi, _mm256_and_si256(_mm256_srli_epi16(v, 4), nibble));
                let none = _mm256_cmpeq_epi8(_mm256_and_si256(l, h), zero);
                marks += (!_mm256_movemask_epi8(none) as u32).count_ones() as usize;
                lines += (_mm256_movemask_epi8(_mm256_cmpeq_epi8(v, newline)) as u32).count_ones()
                    as usize;
            }
        }
        (marks, lines)
    }
}

/// Which classifier [`Set::classify`] runs here, for the results.
pub fn classifier() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        return "avx2";
    }
    "scalar"
}

/// The text a layout wraps: every run's and code block's, in order.
pub fn text_of(doc: &Doc) -> Vec<&str> {
    fn inlines<'a>(i: &'a Inlines, out: &mut Vec<&'a str>) {
        out.extend(i.runs.iter().map(|r| &*r.text));
    }
    fn walk<'a>(blocks: &'a [ada::doc::Block], out: &mut Vec<&'a str>) {
        for block in blocks {
            match &block.kind {
                BlockKind::Heading { inlines: i, .. } | BlockKind::Paragraph(i) => inlines(i, out),
                BlockKind::List(list) => list.items.iter().for_each(|item| walk(&item.blocks, out)),
                BlockKind::Quote { blocks, .. } => walk(blocks, out),
                BlockKind::Code(code) => out.push(&code.text),
                BlockKind::Table(t) => {
                    for row in std::iter::once(&t.header).chain(&t.rows) {
                        row.cells.iter().for_each(|c| inlines(c, out));
                    }
                }
                BlockKind::Image { alt, .. } => out.push(alt),
                BlockKind::Html(html) => out.push(html),
                BlockKind::Rule | BlockKind::FootnoteDefs => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    for note in &doc.footnotes {
        walk(&note.blocks, &mut out);
    }
    out
}

/// The text a layout wraps, copied into one buffer: a floor shouldn't pay for where the IR's
/// runs happen to lie on the heap, which moved the kernel 2× between runs of one build.
pub struct Text {
    text: String,
    runs: Vec<(u32, u32)>,
}

impl Text {
    pub fn new(runs: &[&str]) -> Self {
        let mut text = String::with_capacity(runs.iter().map(|r| r.len()).sum());
        let runs = runs
            .iter()
            .map(|r| {
                let start = text.len() as u32;
                text.push_str(r);
                (start, text.len() as u32)
            })
            .collect();
        Self { text, runs }
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }
}

/// C1 for layout: every run's width, as `wrap::text_width` takes it (its length when ASCII,
/// unicode-width's otherwise), and every break opportunity (a space, `-` or `/`, all ASCII, so
/// counted by byte in UTF-8 too).
pub fn widths(text: &Text) -> (usize, usize) {
    let (mut width, mut breaks) = (0, 0);
    for &(start, end) in &text.runs {
        let t = &text.text[start as usize..end as usize];
        width += if t.is_ascii() { t.len() } else { t.width() };
        breaks += t
            .bytes()
            .map(|b| usize::from(b == b' ') + usize::from(b == b'-') + usize::from(b == b'/'))
            .sum::<usize>();
    }
    (width, breaks)
}

/// `doc::parse`'s options, which its module keeps private: keep the two the same.
pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

/// C2: pulldown-cmark's events with ada's options and offsets, and no IR. ada's parse can't beat
/// it while it keeps pulldown-cmark.
pub fn events(text: &str) -> usize {
    let mut n = 0;
    for event in Parser::new_ext(text, options()).into_offset_iter() {
        black_box(&event);
        n += 1;
    }
    n
}
