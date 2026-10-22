//! Spike C: parse cost.
//!
//! Times pulldown-cmark (with offsets) plus construction of an IR shaped like the one in
//! ada-md-architecture §2: owned, markup-free runs with style bits, a link table, anchors and a
//! source line on every block and run. The IR here is a stand-in; P1 replaces it with the real one.
//!
//! ```zsh
//! cargo run --release --example spike_parse -- ~/opt/birb-vault --min-kb 100
//! ```

use std::time::{Duration, Instant};
use std::{fs, process};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

mod common;

use common::{collect_markdown, markdown_args, ms};

const ITERATIONS: usize = 25;
const BUDGET: Duration = Duration::from_millis(5);

fn main() {
    let (roots, min_kb) = markdown_args(usage);

    let mut files = Vec::new();
    for root in &roots {
        collect_markdown(root, &mut files);
    }
    files.retain(|p| fs::metadata(p).is_ok_and(|m| m.len() >= min_kb * 1024));
    files.sort_by_key(|p| fs::metadata(p).map_or(0, |m| m.len()));

    println!(
        "{:>8} {:>7} {:>7} {:>7} {:>8} {:>8} {:>8}  file",
        "bytes", "blocks", "runs", "links", "min ms", "med ms", "max ms"
    );
    let mut over_budget = 0;
    for path in &files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };

        let mut times = Vec::with_capacity(ITERATIONS);
        let mut doc = build(&text);
        for _ in 0..ITERATIONS {
            let start = Instant::now();
            doc = std::hint::black_box(build(&text));
            times.push(start.elapsed());
        }
        times.sort();
        let median = times[ITERATIONS / 2];
        if median > BUDGET {
            over_budget += 1;
        }
        println!(
            "{:>8} {:>7} {:>7} {:>7} {:>8.3} {:>8.3} {:>8.3}  {}{}",
            text.len(),
            doc.block_count(),
            doc.run_count(),
            doc.links.len(),
            ms(times[0]),
            ms(median),
            ms(times[ITERATIONS - 1]),
            path.display(),
            if median > BUDGET { "  OVER BUDGET" } else { "" },
        );
    }
    println!(
        "\n{} files, {over_budget} over the {BUDGET:?} median budget",
        files.len()
    );
}

fn usage() -> ! {
    eprintln!("usage: spike_parse [--min-kb N] <file-or-dir>...");
    process::exit(2);
}

// --- a stand-in IR ---------------------------------------------------------------------------

#[allow(dead_code)]
struct Doc {
    blocks: Vec<Block>,
    links: Vec<Box<str>>,
    anchors: Vec<Box<str>>,
    line_starts: Vec<u32>,
}

impl Doc {
    fn block_count(&self) -> usize {
        fn count(blocks: &[Block]) -> usize {
            blocks.iter().map(|b| 1 + count(&b.children)).sum()
        }
        count(&self.blocks)
    }

    fn run_count(&self) -> usize {
        fn count(blocks: &[Block]) -> usize {
            blocks
                .iter()
                .map(|b| b.runs.len() + count(&b.children))
                .sum()
        }
        count(&self.blocks)
    }
}

#[allow(dead_code)]
struct Block {
    kind: BlockKind,
    src: (u32, u32),
    runs: Vec<Run>,
    children: Vec<Block>,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum BlockKind {
    Heading(u8),
    Paragraph,
    List,
    Item,
    Quote,
    Code,
    Table,
    Row,
    Cell,
    Rule,
    Html,
    Footnote,
    Other,
}

#[allow(dead_code)]
struct Run {
    text: Box<str>,
    style: u8,
    link: Option<u32>,
    src_line: u32,
}

const EMPH: u8 = 1;
const STRONG: u8 = 2;
const STRIKE: u8 = 4;
const CODE: u8 = 8;

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

fn build(text: &str) -> Doc {
    let line_starts: Vec<u32> = std::iter::once(0)
        .chain(memchr_newlines(text).map(|i| i as u32 + 1))
        .collect();
    let line_of = |offset: usize| line_starts.partition_point(|&s| s as usize <= offset) as u32;

    let mut stack: Vec<Block> = vec![Block {
        kind: BlockKind::Other,
        src: (1, 1),
        runs: Vec::new(),
        children: Vec::new(),
    }];
    let mut links: Vec<Box<str>> = Vec::new();
    let mut anchors: Vec<Box<str>> = Vec::new();
    let mut style = 0u8;
    let mut link: Option<u32> = None;

    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        let line = line_of(range.start);
        match event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::Paragraph => BlockKind::Paragraph,
                    Tag::Heading { level, .. } => BlockKind::Heading(level as u8),
                    Tag::BlockQuote(_) => BlockKind::Quote,
                    Tag::CodeBlock(CodeBlockKind::Fenced(_) | CodeBlockKind::Indented) => {
                        BlockKind::Code
                    }
                    Tag::HtmlBlock => BlockKind::Html,
                    Tag::List(_) => BlockKind::List,
                    Tag::Item => BlockKind::Item,
                    Tag::FootnoteDefinition(_) => BlockKind::Footnote,
                    Tag::Table(_) => BlockKind::Table,
                    Tag::TableHead | Tag::TableRow => BlockKind::Row,
                    Tag::TableCell => BlockKind::Cell,
                    Tag::Emphasis => {
                        style |= EMPH;
                        continue;
                    }
                    Tag::Strong => {
                        style |= STRONG;
                        continue;
                    }
                    Tag::Strikethrough => {
                        style |= STRIKE;
                        continue;
                    }
                    Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                        link = Some(links.len() as u32);
                        links.push(dest_url.into_string().into_boxed_str());
                        continue;
                    }
                    _ => BlockKind::Other,
                };
                let end = line_of(range.end.saturating_sub(1));
                stack.push(Block {
                    kind,
                    src: (line, end),
                    runs: Vec::new(),
                    children: Vec::new(),
                });
            }
            Event::End(end) => match end {
                TagEnd::Emphasis => style &= !EMPH,
                TagEnd::Strong => style &= !STRONG,
                TagEnd::Strikethrough => style &= !STRIKE,
                TagEnd::Link | TagEnd::Image => link = None,
                _ => {
                    let block = stack.pop().expect("unbalanced events");
                    if let BlockKind::Heading(_) = block.kind {
                        let title: String = block.runs.iter().map(|r| &*r.text).collect();
                        anchors.push(title.into_boxed_str());
                    }
                    stack.last_mut().expect("root block").children.push(block);
                }
            },
            Event::Text(t) => push_run(&mut stack, &t, style, link, line),
            Event::Code(t) => push_run(&mut stack, &t, style | CODE, link, line),
            Event::Html(t) | Event::InlineHtml(t) => push_run(&mut stack, &t, style, link, line),
            Event::FootnoteReference(t) => push_run(&mut stack, &t, style, link, line),
            Event::SoftBreak => push_run(&mut stack, " ", style, link, line),
            Event::HardBreak => push_run(&mut stack, "\n", style, link, line),
            Event::Rule => stack.last_mut().expect("root block").children.push(Block {
                kind: BlockKind::Rule,
                src: (line, line),
                runs: Vec::new(),
                children: Vec::new(),
            }),
            Event::TaskListMarker(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    let root = stack.pop().expect("root block");
    Doc {
        blocks: root.children,
        links,
        anchors,
        line_starts,
    }
}

fn push_run(stack: &mut [Block], text: &str, style: u8, link: Option<u32>, src_line: u32) {
    let block = stack.last_mut().expect("root block");
    block.runs.push(Run {
        text: text.into(),
        style,
        link,
        src_line,
    });
}

fn memchr_newlines(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.bytes()
        .enumerate()
        .filter(|&(_, b)| b == b'\n')
        .map(|(i, _)| i)
}
