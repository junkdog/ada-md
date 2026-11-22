//! Properties (ADR-46, OBS-010): a note's front matter ([`super::front`]) read into the key and
//! value rows ada shows above it. YAML is read by a small reader of ada's own, without a new
//! dependency, for the YAML that Obsidian and static site generators write: `key: value` at the
//! block's left edge, a value plain or quoted, block (`- a`) and flow (`[a, b]`) lists, block
//! scalars (`|`, `>`) and comments. A row shows what it can on one line: a nested map, or a
//! literal of several lines, is its first line cut with `…`; a value in a form it doesn't read
//! (an anchor, a tag) is its text as written. A block that isn't a map of keys at all (a line at
//! the left edge that isn't a key) is unreadable, and shows as written, as Obsidian falls back to
//! source. TOML (`+++`) is read by the `toml` crate ada already has for its config.

use std::borrow::Cow;
use std::ops::Range;

use toml::de::{DeTable, DeValue};

use super::front::{self, Lang};

/// A property as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prop<'a> {
    pub key: Cow<'a, str>,
    pub value: Value<'a>,
    /// Its lines, from 0 at the block's first.
    pub lines: Range<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    /// No value, or YAML's null (`~`, `null`).
    Empty,
    Text(Cow<'a, str>),
    /// A list's items: one that can't show whole is cut, ending in `…`.
    List(Vec<Cow<'a, str>>),
    /// The first line of what a row can't show whole: a nested map, a literal of several lines.
    /// Shown with `…` after it.
    Cut(Cow<'a, str>),
}

/// The properties `body` (a front matter block's text, its fences left out) holds, in order;
/// `None` when it can't be read.
pub fn read(body: &str, lang: Lang) -> Option<Vec<Prop<'_>>> {
    match lang {
        Lang::Yaml => yaml(body),
        Lang::Toml => toml(body),
    }
}

/// Keys of Obsidian's and its plugins' own, not shown as rows: the note's CSS classes, and the
/// markers of a Kanban board and an Excalidraw drawing (ADR-46). They stay in the source.
pub fn is_hidden(key: &str) -> bool {
    matches!(key, "cssclasses" | "cssclass" | "kanban-plugin") || key.starts_with("excalidraw-")
}

/// Keys whose values are tags (OBS-006), drawn as tags.
pub fn is_tags(key: &str) -> bool {
    matches!(key, "tags" | "tag")
}

/// Keys whose values are other names for the note, which a `[[link]]` may use (OBS-010).
pub fn is_aliases(key: &str) -> bool {
    matches!(key, "aliases" | "alias")
}

/// A value's items: a list's, or a text's split at commas (and, for tags, spaces too), as
/// Obsidian once read them. Empty ones are left out.
pub fn items<'v>(value: &'v Value<'_>, tags: bool) -> Vec<&'v str> {
    let split = |c: char| c == ',' || (tags && c.is_whitespace());
    let items: Vec<&str> = match value {
        Value::Empty | Value::Cut(_) => Vec::new(),
        Value::Text(t) => t.split(split).collect(),
        Value::List(items) => items.iter().map(|i| &**i).collect(),
    };
    items
        .into_iter()
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .collect()
}

/// The aliases the front matter of a note's `text` gives it: `aliases`, or the older `alias`.
pub fn aliases(text: &str) -> Vec<String> {
    let Some(front) = front::read(text, true) else {
        return Vec::new();
    };
    let Some(props) = read(&text[front.body], front.lang) else {
        return Vec::new();
    };
    props
        .iter()
        .filter(|p| is_aliases(&p.key))
        .flat_map(|p| items(&p.value, false))
        .map(str::to_owned)
        .collect()
}

// --- YAML ------------------------------------------------------------------------------------

fn yaml(body: &str) -> Option<Vec<Prop<'_>>> {
    let lines: Vec<&str> = body
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut props = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_blank_or_comment(line) {
            i += 1;
            continue;
        }
        if is_indented(line) || is_item(line) {
            return None;
        }
        let (key, rest) = key_of(line)?;
        // its lines go on while they're indented, or items of a list at the left edge; blank
        // lines and comments between them are in only if more of it follows
        let mut last = i;
        let mut j = i + 1;
        while let Some(l) = lines.get(j) {
            if l.trim().is_empty() || (is_blank_or_comment(l) && !is_indented(l)) {
                j += 1;
                continue;
            }
            if is_indented(l) || is_item(l) {
                last = j;
                j += 1;
                continue;
            }
            break;
        }
        // a block scalar's lines are its text, comments and all
        let more = &lines[i + 1..=last];
        props.push(Prop {
            key,
            value: value(rest, more),
            lines: i as u32..last as u32 + 1,
        });
        i = last + 1;
    }
    Some(props)
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

fn is_indented(line: &str) -> bool {
    line.starts_with([' ', '\t'])
}

/// A list's item: `-` alone or before a space.
fn is_item(line: &str) -> bool {
    let t = line.trim_start();
    t == "-" || t.starts_with("- ") || t.starts_with("-\t")
}

/// A line's key and what follows its `:`, trimmed, without a comment.
fn key_of(line: &str) -> Option<(Cow<'_, str>, &str)> {
    let (key, after) = if line.starts_with(['"', '\'']) {
        let (key, len) = quoted(line)?;
        (key, line[len..].trim_start().strip_prefix(':')?)
    } else {
        let b = line.as_bytes();
        let at = (0..b.len())
            .find(|&k| b[k] == b':' && matches!(b.get(k + 1), None | Some(b' ' | b'\t')))?;
        (Cow::Borrowed(line[..at].trim_end()), &line[at + 1..])
    };
    if key.is_empty() || !(after.is_empty() || after.starts_with([' ', '\t'])) {
        return None;
    }
    let after = after.trim();
    Some((key, if after.starts_with('#') { "" } else { after }))
}

/// A value: `rest`, what follows the key on its line, and `more`, its lines after.
fn value<'a>(rest: &'a str, more: &[&'a str]) -> Value<'a> {
    // its lines that aren't blank, trimmed, comments left out
    let lines = || {
        more.iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
    };
    let Some(&first) = rest.as_bytes().first() else {
        return match lines().next() {
            None => Value::Empty,
            Some(line) if is_item(line) => block_list(more),
            Some(line) => Value::Cut(Cow::Borrowed(line)),
        };
    };
    let one_line = lines().next().is_none();
    match first {
        b'|' | b'>' if is_block_scalar(rest) => block_scalar(first == b'>', more),
        b'[' => {
            if let Some(links) = bare_links(rest) {
                return links;
            }
            let joined = joined(rest, more, one_line);
            match flow_list(&joined) {
                Some(items) => Value::List(items.into_iter().map(|i| rebase(&joined, i)).collect()),
                None => Value::Cut(Cow::Borrowed(rest)),
            }
        }
        b'"' | b'\'' => {
            let joined = joined(rest, more, one_line);
            match quoted(&joined) {
                Some((text, _)) => Value::Text(rebase(&joined, text)),
                None => Value::Cut(Cow::Borrowed(rest)),
            }
        }
        // a flow map, an anchor, an alias or a tag: shown as written
        b'{' | b'&' | b'*' | b'!' => match one_line {
            true => Value::Text(Cow::Borrowed(rest)),
            false => Value::Cut(Cow::Borrowed(rest)),
        },
        _ => {
            let text = plain(rest);
            if one_line {
                return match text {
                    "~" | "null" | "Null" | "NULL" => Value::Empty,
                    _ => Value::Text(Cow::Borrowed(text)),
                };
            }
            // a plain scalar over several lines folds into one
            let mut out = text.to_owned();
            for line in lines() {
                out.push(' ');
                out.push_str(plain(line));
            }
            Value::Text(Cow::Owned(out))
        }
    }
}

/// `rest`, and when the value goes on past its line, its other lines joined with spaces: how
/// YAML folds a flow collection or a quoted string over several lines.
fn joined<'a>(rest: &'a str, more: &[&str], one_line: bool) -> Cow<'a, str> {
    if one_line {
        return Cow::Borrowed(rest);
    }
    let mut out = rest.to_owned();
    for line in more.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        out.push(' ');
        out.push_str(line);
    }
    Cow::Owned(out)
}

/// `piece`, a piece of `source`, as long-lived as `source`: borrowed from the note where
/// `source` is.
fn rebase<'a>(source: &Cow<'a, str>, piece: Cow<'_, str>) -> Cow<'a, str> {
    if let (Cow::Borrowed(s), Cow::Borrowed(p)) = (source, &piece) {
        let at = (p.as_ptr() as usize).wrapping_sub(s.as_ptr() as usize);
        if let Some(same) = s
            .get(at..at.wrapping_add(p.len()))
            .filter(|same| same.as_ptr() == p.as_ptr())
        {
            return Cow::Borrowed(same);
        }
    }
    Cow::Owned(piece.into_owned())
}

/// A plain scalar without a comment after it (` #`).
fn plain(text: &str) -> &str {
    let b = text.as_bytes();
    let end = (1..b.len())
        .find(|&k| b[k] == b'#' && matches!(b[k - 1], b' ' | b'\t'))
        .unwrap_or(b.len());
    text[..end].trim()
}

/// `|` or `>`, then a chomping sign and an indent's digit in either order, then a comment.
fn is_block_scalar(rest: &str) -> bool {
    let tail =
        rest[1..].trim_start_matches(['+', '-', '1', '2', '3', '4', '5', '6', '7', '8', '9']);
    let tail = tail.trim();
    tail.is_empty() || tail.starts_with('#')
}

/// A block scalar's lines: folded (`>`) into one, a literal (`|`) shown whole when it has one.
fn block_scalar<'a>(folded: bool, more: &[&'a str]) -> Value<'a> {
    let lines: Vec<&str> = more
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    match (&lines[..], folded) {
        ([], _) => Value::Empty,
        ([one], _) => Value::Text(Cow::Borrowed(one)),
        (lines, true) => Value::Text(Cow::Owned(lines.join(" "))),
        ([first, ..], false) => Value::Cut(Cow::Borrowed(first)),
    }
}

/// A block list's items (`- a` lines). An item with more lines under it than its own (a map, or
/// a scalar over several lines) is cut.
fn block_list<'a>(more: &[&'a str]) -> Value<'a> {
    let mut items: Vec<Cow<'a, str>> = Vec::new();
    let mut cut = false;
    let close = |items: &mut Vec<Cow<'a, str>>, cut: &mut bool| {
        if std::mem::take(cut)
            && let Some(last) = items.last_mut()
        {
            last.to_mut().push_str(" …");
        }
    };
    let mut indent = None;
    for line in more {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let at = line.len() - t.len();
        if is_item(line) && indent.is_none_or(|i| at <= i) {
            close(&mut items, &mut cut);
            indent = Some(at);
            items.push(scalar(t[1..].trim()));
        } else if let Some(last) = items.last_mut().filter(|l| l.is_empty()) {
            // `-` alone, its value on the line under it
            *last = scalar(t);
        } else {
            cut = true;
        }
    }
    close(&mut items, &mut cut);
    items.retain(|i| !i.is_empty());
    Value::List(items)
}

/// An item's scalar: quoted, or plain without its comment.
fn scalar(text: &str) -> Cow<'_, str> {
    if text.starts_with(['"', '\''])
        && let Some((s, len)) = quoted(text)
        && empty_or_comment(&text[len..])
    {
        return s;
    }
    Cow::Borrowed(plain(text))
}

/// Nothing but spaces and a comment.
fn empty_or_comment(text: &str) -> bool {
    let text = text.trim();
    text.is_empty() || text.starts_with('#')
}

/// `[[a]]`, or `[[a]], [[b]]`, written without quotes: in YAML a list in a list, but what was
/// meant is links, as Obsidian writes them quoted (`"[[a]]"`).
fn bare_links(rest: &str) -> Option<Value<'_>> {
    let rest = plain(rest);
    let mut items = Vec::new();
    let mut s = rest;
    loop {
        let inner = s.strip_prefix("[[")?;
        let end = inner.find("]]")?;
        if inner[..end].contains(['[', ']']) || end == 0 {
            return None;
        }
        items.push(Cow::Borrowed(&s[..end + 4]));
        s = inner[end + 2..].trim_start();
        match s.strip_prefix(',') {
            Some(next) => s = next.trim_start(),
            None if s.is_empty() => break,
            None => return None,
        }
    }
    Some(match items.len() {
        1 => Value::Text(items.remove(0)),
        _ => Value::List(items),
    })
}

/// A flow list's items, `[a, "b", [c]]`: quoted ones unquoted, a nested list or map as written.
/// `None` if it doesn't close, or something other than a comment follows it.
fn flow_list(text: &str) -> Option<Vec<Cow<'_, str>>> {
    let b = text.as_bytes();
    debug_assert_eq!(b.first(), Some(&b'['));
    let mut items = Vec::new();
    let mut depth = 0;
    let mut start = 1;
    let mut k = 1;
    fn item<'t>(items: &mut Vec<Cow<'t, str>>, piece: &'t str) {
        let piece = piece.trim();
        if piece.is_empty() {
            return;
        }
        items.push(match piece.as_bytes()[0] {
            b'"' | b'\'' => quoted(piece).map_or(Cow::Borrowed(piece), |(s, _)| s),
            _ => Cow::Borrowed(piece),
        });
    }
    while k < b.len() {
        match b[k] {
            b'"' | b'\'' if text[start..k].trim().is_empty() => {
                let (_, len) = quoted(&text[k..])?;
                k += len;
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' if depth > 0 => depth -= 1,
            b']' => {
                item(&mut items, &text[start..k]);
                return empty_or_comment(&text[k + 1..]).then_some(items);
            }
            b',' if depth == 0 => {
                item(&mut items, &text[start..k]);
                start = k + 1;
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// The quoted string `text` starts with, unescaped, and the bytes it takes, quotes included;
/// `None` if it doesn't close.
fn quoted(text: &str) -> Option<(Cow<'_, str>, usize)> {
    let quote = *text.as_bytes().first()?;
    let body = &text[1..];
    if quote == b'\'' {
        // `''` is a quote
        let mut k = 0;
        let b = body.as_bytes();
        let mut escaped = false;
        while k < b.len() {
            if b[k] == b'\'' {
                if b.get(k + 1) == Some(&b'\'') {
                    escaped = true;
                    k += 2;
                    continue;
                }
                let s = &body[..k];
                let s = match escaped {
                    true => Cow::Owned(s.replace("''", "'")),
                    false => Cow::Borrowed(s),
                };
                return Some((s, k + 2));
            }
            k += 1;
        }
        return None;
    }
    let mut out: Option<String> = None;
    let mut chars = body.char_indices();
    while let Some((k, c)) = chars.next() {
        match c {
            '"' => {
                let s = out.map_or(Cow::Borrowed(&body[..k]), Cow::Owned);
                return Some((s, k + 2));
            }
            '\\' => {
                let o = out.get_or_insert_with(|| body[..k].to_owned());
                let (_, e) = chars.next()?;
                let hex = |n: usize, chars: &mut std::str::CharIndices<'_>| {
                    let digits: String = chars.by_ref().take(n).map(|(_, c)| c).collect();
                    u32::from_str_radix(&digits, 16)
                        .ok()
                        .and_then(char::from_u32)
                };
                match e {
                    'n' | 't' | 'r' | '_' | 'L' | 'P' => o.push(' '),
                    'x' => o.extend(hex(2, &mut chars)),
                    'u' => o.extend(hex(4, &mut chars)),
                    'U' => o.extend(hex(8, &mut chars)),
                    '0' | 'a' | 'b' | 'e' | 'f' | 'v' => {}
                    'N' => o.push('\u{85}'),
                    // `\"`, `\\`, `\/`, `\ ` and the rest: the character itself
                    other => o.push(other),
                }
            }
            c => {
                if let Some(o) = &mut out {
                    o.push(c);
                }
            }
        }
    }
    None
}

// --- TOML ------------------------------------------------------------------------------------

fn toml(body: &str) -> Option<Vec<Prop<'_>>> {
    let table = DeTable::parse(body).ok()?.into_inner();
    // the newlines before `at`, found once: counted from the start for each key, many keys were
    // quadratic (PAR-013)
    let newlines: Vec<usize> = memchr::memchr_iter(b'\n', body.as_bytes()).collect();
    let line = |at: usize| newlines.partition_point(|&nl| nl < at) as u32;
    // the table is sorted by key: back into the order they're written in
    let mut entries: Vec<_> = table.into_iter().collect();
    entries.sort_by_key(|(k, _)| k.span().start);
    let props = entries
        .into_iter()
        .map(|(k, v)| {
            let start = k.span().start;
            let end = end_of(&v).max(start + 1);
            let raw = |v: &toml::Spanned<DeValue<'_>>| one_line(&body[v.span()]);
            let value = match v.get_ref() {
                DeValue::String(s) => Value::Text(s.clone()),
                DeValue::Array(items) => Value::List(
                    items
                        .iter()
                        .map(|i| match i.get_ref() {
                            DeValue::String(s) => s.clone(),
                            _ => raw(i),
                        })
                        .collect(),
                ),
                DeValue::Table(t) => {
                    let first = t.iter().min_by_key(|(k, _)| k.span().start);
                    match first {
                        Some((k, v)) => {
                            let entry = format!("{} = {}", k.get_ref(), raw(v));
                            let whole = t.len() == 1 && !matches!(v.get_ref(), DeValue::Table(_));
                            match whole {
                                true => Value::Text(Cow::Owned(entry)),
                                false => Value::Cut(Cow::Owned(entry)),
                            }
                        }
                        None => Value::Empty,
                    }
                }
                _ => Value::Text(raw(&v)),
            };
            Prop {
                key: k.into_inner(),
                value,
                lines: line(start)..line(end - 1) + 1,
            }
        })
        .collect();
    Some(props)
}

/// Where a value's text ends, a table's last entry's included.
fn end_of(v: &toml::Spanned<DeValue<'_>>) -> usize {
    match v.get_ref() {
        DeValue::Table(t) => t
            .iter()
            .map(|(k, v)| end_of(v).max(k.span().end))
            .max()
            .unwrap_or(v.span().end)
            .max(v.span().end),
        _ => v.span().end,
    }
}

/// `text`'s first line, cut with `…` when more follow.
fn one_line(text: &str) -> Cow<'_, str> {
    match text.split_once('\n') {
        None => Cow::Borrowed(text.trim()),
        Some((first, _)) => Cow::Owned(format!("{} …", first.trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(body: &str) -> Vec<(String, Value<'_>)> {
        read(body, Lang::Yaml)
            .unwrap()
            .into_iter()
            .map(|p| (p.key.into_owned(), p.value))
            .collect()
    }

    fn text(s: &str) -> Value<'_> {
        Value::Text(Cow::Borrowed(s))
    }

    fn list<'a>(items: &[&'a str]) -> Value<'a> {
        Value::List(items.iter().map(|&i| Cow::Borrowed(i)).collect())
    }

    #[test]
    fn scalars_plain_and_quoted() {
        let body = "title: A note # a comment\nsingle: 'it''s'\ndouble: \"a \\\"b\\\" \\\\ c\"\n\
                    date: 2024-01-02\nempty:\nnull: ~\n\"quoted key\": yes\nurl: https://x.org/#a\n";
        assert_eq!(
            yaml(body),
            [
                ("title".into(), text("A note")),
                ("single".into(), text("it's")),
                ("double".into(), text("a \"b\" \\ c")),
                ("date".into(), text("2024-01-02")),
                ("empty".into(), Value::Empty),
                ("null".into(), Value::Empty),
                ("quoted key".into(), text("yes")),
                // a `#` not after a space opens no comment
                ("url".into(), text("https://x.org/#a")),
            ]
        );
    }

    #[test]
    fn lists_block_and_flow() {
        let body = "tags:\n  - a\n  - \"b c\"\n# between\n  - d # comment\nflow: [x, 'y, z', [n]]\n\
                    edge:\n- one\n- two\nlinks: [[A]], [[B|b]]\nlink: [[C]]\nquoted: \"[[D]]\"\n";
        assert_eq!(
            yaml(body),
            [
                ("tags".into(), list(&["a", "b c", "d"])),
                ("flow".into(), list(&["x", "y, z", "[n]"])),
                ("edge".into(), list(&["one", "two"])),
                ("links".into(), list(&["[[A]]", "[[B|b]]"])),
                ("link".into(), text("[[C]]")),
                ("quoted".into(), text("[[D]]")),
            ]
        );
    }

    #[test]
    fn what_a_row_cant_show_whole_is_cut() {
        let body = "author:\n  name: Ada\n  url: x\nabstract: |\n  one\n  two\nsummary: >\n  folded\n  \
                    lines\nlong: plain\n  goes on\npeople:\n  - name: a\n    role: b\n  - c\n";
        assert_eq!(
            yaml(body),
            [
                ("author".into(), Value::Cut(Cow::Borrowed("name: Ada"))),
                ("abstract".into(), Value::Cut(Cow::Borrowed("one"))),
                ("summary".into(), text("folded lines")),
                ("long".into(), text("plain goes on")),
                (
                    "people".into(),
                    Value::List(vec![Cow::Owned("name: a …".into()), Cow::Borrowed("c")])
                ),
            ]
        );
    }

    #[test]
    fn each_property_knows_its_lines() {
        let body = "a: 1\n\nb:\n  - x\n\n  - y\n\n# c\nc: 2\n";
        let lines: Vec<_> = read(body, Lang::Yaml)
            .unwrap()
            .into_iter()
            .map(|p| p.lines)
            .collect();
        assert_eq!(lines, [0..1, 2..6, 8..9]);
    }

    #[test]
    fn a_block_that_isnt_a_map_is_unreadable() {
        for body in [
            "- a\n- b\n",
            "just text\n",
            "  indented: x\n",
            "a: 1\nnot a key\n",
        ] {
            assert_eq!(read(body, Lang::Yaml), None, "{body:?}");
        }
        // comments alone are a map without keys
        assert_eq!(read("# Title\n", Lang::Yaml), Some(Vec::new()));
    }

    #[test]
    fn toml_in_the_order_written() {
        let body =
            "title = \"Hi\"\ndraft = true\ntags = [\"a\", \"b\"]\n\n[params]\nx = 1\ny = 2\n";
        let props = read(body, Lang::Toml).unwrap();
        let got: Vec<_> = props
            .iter()
            .map(|p| (&*p.key, &p.value, &p.lines))
            .collect();
        assert_eq!(
            got,
            [
                ("title", &text("Hi"), &(0..1)),
                ("draft", &text("true"), &(1..2)),
                ("tags", &list(&["a", "b"]), &(2..3)),
                ("params", &Value::Cut(Cow::Owned("x = 1".into())), &(4..7)),
            ]
        );
        assert_eq!(read("title = \n", Lang::Toml), None);
    }

    #[test]
    fn aliases_and_items() {
        let note = "---\naliases: [One, \"Two\"]\nalias: Three, Four\n---\nbody";
        assert_eq!(aliases(note), ["One", "Two", "Three", "Four"]);
        assert_eq!(aliases("no front matter"), Vec::<String>::new());
        assert_eq!(items(&text("a b, c"), true), ["a", "b", "c"]);
    }
}
