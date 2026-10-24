//! Documents that aren't markdown: a code or text file becomes one code block, and an image file
//! one image block, so that everything downstream treats them the way it treats a note.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use super::parse::line_starts;
use super::{Block, BlockKind, CodeBlock, Doc, LineRange, LinkKind, LinkRef, UnitId, code_lines};

/// A whole file of code in `lang`, or of plain text when there is none. A mermaid file draws
/// its diagram, as a mermaid block in a note does.
pub fn code(text: &str, lang: Option<&str>) -> Doc {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    let lines = code_lines(text).count() as UnitId;
    Doc {
        blocks: vec![Block {
            kind: BlockKind::Code(CodeBlock {
                id: 0,
                lang: lang.map(Into::into),
                text: text.into(),
                hash: h.finish(),
                first_line: 1,
                fenced: false,
                first_unit: 0,
                diagram: super::diagram(lang, text),
            }),
            src: LineRange::new(1, lines),
        }],
        line_starts: line_starts(text),
        code_count: 1,
        unit_count: lines,
        ..Doc::default()
    }
}

/// A piece of a longer file of code or text, printed a piece at a time (O-52): its lines, the
/// first of them line `first_line` of the file. `hash` stands in for a hash of the text, which
/// only has to tell the pieces apart.
pub fn code_piece(text: Arc<str>, lang: Option<&str>, first_line: u32, hash: u64) -> Doc {
    let lines = code_lines(&text).count() as UnitId;
    Doc {
        blocks: vec![Block {
            kind: BlockKind::Code(CodeBlock {
                id: 0,
                lang: lang.map(Into::into),
                text,
                hash,
                first_line,
                fenced: false,
                first_unit: 0,
                diagram: None,
            }),
            src: LineRange::new(first_line, first_line + lines.max(1) - 1),
        }],
        code_count: 1,
        unit_count: lines,
        ..Doc::default()
    }
}

/// An image file: the document's one image, captioned with the file's `name`.
pub fn image(name: &str) -> Doc {
    Doc {
        blocks: vec![Block {
            kind: BlockKind::Image {
                link: 0,
                alt: name.into(),
                unit: 0,
            },
            src: LineRange::NONE,
        }],
        links: vec![LinkRef {
            dest: dest(name).into(),
            title: "".into(),
            kind: LinkKind::Image,
            src_line: 0,
        }],
        line_starts: vec![0],
        unit_count: 1,
        ..Doc::default()
    }
}

/// A link to the file `name` next to the document, escaped so that it reads back as that name
/// rather than as a fragment or a scheme.
fn dest(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            ':' => out.push_str("%3A"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::{LocalTarget, Target};

    #[test]
    fn a_code_file_is_one_block_from_line_one() {
        let doc = code("fn main() {}\n\nlet x;\n", Some("Rust"));
        let [block] = &doc.blocks[..] else {
            panic!("{:?}", doc.blocks);
        };
        let BlockKind::Code(code) = &block.kind else {
            panic!("{block:?}");
        };
        assert_eq!((code.first_line, code.fenced), (1, false));
        assert_eq!(code.lang.as_deref(), Some("Rust"));
        assert_eq!(block.src, LineRange::new(1, 3));
        assert_eq!(
            (doc.code_count, doc.unit_count, doc.line_count()),
            (1, 3, 3)
        );
        assert_eq!(super::code("", None).unit_count, 0);
    }

    #[test]
    fn an_image_links_to_its_own_file() {
        for name in ["pic.png", "50% #2.png", "c:d.jpg"] {
            let doc = image(name);
            assert_eq!(
                Target::of(&doc.links[0]),
                Target::Local {
                    target: LocalTarget::Path(name.into()),
                    frag: None,
                },
                "{name}"
            );
        }
    }
}
