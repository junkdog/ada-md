//! A man page's sections and subsections (X-188): what `o` lists in place of headings, as a
//! diff's files are listed ([`super::diffs`]).

/// A section's heading at the margin, or a subsection's three columns in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    /// The line (from 1) its heading is on.
    pub line: u32,
    /// 1 for a section, 2 for a subsection.
    pub level: u8,
    pub name: String,
}

/// Every section and subsection of the man page `text`, in order, as its scanner reads them
/// ([`ada_parse::man_sections`]): not the title or the footer.
pub fn chapters(text: &str) -> Vec<Chapter> {
    ada_parse::man_sections(text)
        .into_iter()
        .map(|s| Chapter {
            line: s.line.saturating_add(1),
            level: s.level,
            name: s.text.to_owned(),
        })
        .collect()
}
