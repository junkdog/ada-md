//! Front matter: a YAML block at the very start of a note, from a `---` line to a `---` or `...`
//! line, or a TOML one between `+++` lines (Hugo's), found here rather than by pulldown-cmark,
//! whose option for it opens a block at any `---` that starts a block, anywhere in a note: a rule
//! followed by text then hid everything up to the next `---`. The parse starts past the block
//! this finds, and [`super::props`] reads it into the properties ada shows above the note
//! (ADR-46).
//!
//! It's found as pulldown-cmark found it at the start: the `---` alone on its line (spaces after
//! it allowed), a first line that neither is blank nor closes the block, and a closing line of
//! three `-` or `.` exactly, spaces after them allowed. Past that, Obsidian's reading takes blank
//! lines after the `---` (OBS-011): the Excalidraw plugin writes it so (332 of 412 drawings in the
//! survey), as do a few notes; CommonMark reads those as a rule, then the YAML as a paragraph and
//! a setext heading. Only in Obsidian's dialect (ADR-45: in CommonMark, `---`, a blank line and
//! text are a rule and the text, which a block would hide), but for an Excalidraw drawing's, known
//! by its `excalidraw-plugin:` key, in any. TOML's `+++` is found the same way, closed by a `+++`
//! line, in any dialect: no markdown starts with a line of three `+`.

use std::ops::Range;

/// The front matter a note opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Front {
    /// Past its closing line's line end: where the note's markdown starts.
    pub end: usize,
    /// Its text, from its first line that isn't blank to its closing line.
    pub body: Range<usize>,
    pub lang: Lang,
    /// An Excalidraw drawing's: its `excalidraw-plugin:` key is there.
    pub excalidraw: bool,
    /// Blank lines after its `---`: Obsidian's reading (OBS-011), CommonMark's a rule.
    pub loose: bool,
}

impl Front {
    /// Whether a note in Obsidian's dialect or not, `obsidian`, takes it: a loose one only in
    /// Obsidian's, or an Excalidraw drawing's in any.
    pub fn taken(&self, obsidian: bool) -> bool {
        !self.loose || obsidian || self.excalidraw
    }
}

/// What ada shows of an Excalidraw drawing (OBS-015), whose front matter `front` read: its text
/// past the front matter and the plugin's banner after it (`==⚠  Switch to EXCALIDRAW VIEW…`),
/// up to the drawing's data, from its `%%` line or its `# Drawing` heading on. The plugin writes
/// its text elements and embedded files before both, in all 418 of the survey's drawings.
pub fn drawing(text: &str, front: &Front) -> Range<usize> {
    let b = text.as_bytes();
    let line_end = |at: usize| memchr::memchr(b'\n', &b[at..]).map_or(b.len(), |i| at + i + 1);
    let mut start = front.end;
    while let Some(n) = blank(&b[start..]) {
        start += n;
    }
    start = match text[start..].starts_with("==⚠") {
        true => line_end(start),
        false => front.end,
    };
    let mut line = start;
    while line < b.len() {
        let next = line_end(line);
        let l = text[line..next].trim_end();
        if l == "%%" || l == "# Drawing" || l == "## Drawing" {
            return start..line;
        }
        line = next;
    }
    start..b.len()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Yaml,
    Toml,
}

impl Lang {
    /// Its name as a code block's language.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Yaml => "yaml",
            Lang::Toml => "toml",
        }
    }
}

/// Where the front matter `text` opens with ends: past its closing line's line end. `obsidian`
/// takes blank lines after the `---`.
pub fn find(text: &str, obsidian: bool) -> Option<usize> {
    read(text, obsidian).map(|f| f.end)
}

/// The front matter `text` opens with. `obsidian` takes blank lines after the `---`.
pub fn read(text: &str, obsidian: bool) -> Option<Front> {
    let b = text.as_bytes();
    let (lang, fence) = match b.get(..3)? {
        b"---" => (Lang::Yaml, b'-'),
        b"+++" => (Lang::Toml, b'+'),
        _ => return None,
    };
    let rest = &b[3..];
    let ws = rest
        .iter()
        .take_while(|&&c| c != b'\n' && c.is_ascii_whitespace())
        .count();
    if rest.get(ws) != Some(&b'\n') {
        return None;
    }
    let first = 3 + ws + 1;
    let mut body = first;
    while let Some(n) = blank(&b[body..]) {
        body += n;
    }
    let loose = body > first;
    if body == b.len() {
        return None;
    }
    let mut excalidraw = false;
    let mut line = body;
    while line < b.len() {
        let next = memchr::memchr(b'\n', &b[line..]).map_or(b.len(), |i| line + i + 1);
        let l = &b[line..next];
        if closes(l, fence) {
            let front = Front {
                end: next,
                body: body..line,
                lang,
                excalidraw,
                loose,
            };
            // a block closed on its first line is none
            return (line > body && front.taken(obsidian)).then_some(front);
        }
        excalidraw |= l.starts_with(b"excalidraw-plugin:");
        line = next;
    }
    None
}

/// The length of the blank line `b` starts with, its line end included; `None` if it isn't one,
/// or the text ends on it.
fn blank(b: &[u8]) -> Option<usize> {
    let n = b.iter().take_while(|&&c| matches!(c, b' ' | b'\t')).count();
    match &b[n..] {
        [b'\r', b'\n', ..] => Some(n + 2),
        [b'\n' | b'\r', ..] => Some(n + 1),
        _ => None,
    }
}

/// Whether `line` closes a block fenced by `fence`: three of it exactly, or `...` for YAML's,
/// then spaces.
fn closes(line: &[u8], fence: u8) -> bool {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let three = |c: u8| line.len() >= 3 && line[..3].iter().all(|&b| b == c);
    (three(fence) || (fence == b'-' && three(b'.'))) && line[3..].iter().all(|&c| c == b' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_is_found_at_the_start() {
        let text = "---\ntitle: x\n---\n# Body\n";
        assert_eq!(find(text, false), Some(17));
        assert_eq!(read(text, false).unwrap().body, 4..13);
        assert_eq!(find("---  \r\nt: x\r\n...  \r\nbody", false), Some(20));
        // to the end of a text without a last line end
        assert_eq!(find("---\nt: x\n---", false), Some(12));
    }

    #[test]
    fn a_blank_line_after_the_dashes_is_obsidians() {
        let text = "---\n\ntags: [a]\n---\n# Title\n";
        assert_eq!(find(text, true), Some(19));
        assert_eq!(read(text, true).unwrap().body, 5..15);
        assert_eq!(find(text, false), None);
        // several, and CRLF
        assert_eq!(find("---\r\n  \r\n\r\nkey: v\r\n...\r\n", true), Some(24));
        // an Excalidraw drawing's, in any dialect
        let text = "---\n\nexcalidraw-plugin: parsed\ntags: [excalidraw]\n\n---\n";
        let front = read(text, false).unwrap();
        assert_eq!(front.end, text.len());
        assert!(front.excalidraw);
    }

    #[test]
    fn tomls_pluses_are_front_matter_too() {
        let text = "+++\ntitle = \"x\"\n+++\n# Body\n";
        let front = read(text, false).unwrap();
        assert_eq!((front.end, front.body, front.lang), (20, 4..16, Lang::Toml));
        // closed by its own fence only
        assert_eq!(find("+++\ntitle = \"x\"\n---\n", false), None);
        assert_eq!(find("---\ntitle: x\n+++\n", false), None);
        assert_eq!(find("+++\ntitle = \"x\"\n...\n", false), None);
    }

    #[test]
    fn what_isnt_front_matter_isnt_found() {
        for text in [
            // never closed, closed on its first line, or only blank lines
            "---\ntags: [a]\n",
            "---\n---\n",
            "---\n\ntags: [a]\n",
            "---\n\n---\n",
            "---\n\n\n",
            "---\n\n",
            "---",
            "+++\n+++\n",
            // not three dashes, not alone, or not at the start
            "----\ntags: [a]\n---\n",
            "--- x\ntags: [a]\n---\n",
            "\n---\ntags: [a]\n---\n",
            "intro\n\n---\ntags: [a]\n---\n",
            "++++\nt = 1\n+++\n",
            // a closer of four, or indented
            "---\ntags: [a]\n----\n",
            "---\ntags: [a]\n ---\n",
        ] {
            assert_eq!(find(text, true), None, "{text:?}");
        }
    }

    /// A drawing's text: past the plugin's banner, up to its data (OBS-015).
    #[test]
    fn a_drawings_text_is_between_its_banner_and_its_data() {
        let shown = |text: &str| {
            let front = read(text, false).unwrap();
            assert!(front.excalidraw);
            text[drawing(text, &front)].to_owned()
        };
        let fm = "---\n\nexcalidraw-plugin: parsed\n\n---\n";
        let banner = "==⚠  Switch to EXCALIDRAW VIEW in the MORE OPTIONS menu. ⚠==\n\n\n";
        let text = "# Excalidraw Data\n## Text Elements\nhello ^abcd1234\n\n";
        let data = "%%\n## Drawing\n```compressed-json\nN4Ig\n```\n%%";
        // the blank lines after the banner kept, as the note's
        assert_eq!(
            shown(&format!("{fm}{banner}{text}{data}")),
            format!("\n\n{text}")
        );
        // the older form, its data under `# Drawing` before any `%%`
        let old = "# Text Elements\n# Drawing\n```json\n{}\n```\n%%\n";
        assert_eq!(
            shown(&format!("{fm}{banner}{old}")),
            "\n\n# Text Elements\n"
        );
        // no banner: from the front matter's end, to the text's end with no data
        assert_eq!(shown(&format!("{fm}x\n")), "x\n");
    }
}
