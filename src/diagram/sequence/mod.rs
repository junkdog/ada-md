//! Sequence diagrams: the IR and its line parser. Drawing is in [`draw`](mod@draw).

mod draw;

use anpa::combinators::{map, not_empty, succeed};
use anpa::core::{ParserExtNoState, StrParser};
use anpa::parsers::{item_while, skip};
use anpa::{or, tuplify};

pub use draw::draw;

use super::{MAX_STATEMENTS, Source, Text};

/// The most participants a diagram may have.
const MAX_PARTICIPANTS: usize = 64;

#[derive(Debug, Default)]
pub struct Sequence {
    pub title: Vec<Text>,
    pub participants: Vec<Participant>,
    pub events: Vec<Event>,
}

#[derive(Debug)]
pub struct Participant {
    pub id: String,
    /// The name shown, a line per `<br/>`.
    pub label: Vec<Text>,
    pub actor: bool,
}

pub type Who = u16;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Message(Message),
    Note {
        at: NoteAt,
        text: Vec<Text>,
    },
    /// A frame opens: `loop`, `alt`, `opt`, `par`, `critical` or `break`.
    Begin {
        kind: &'static str,
        label: Text,
    },
    /// A frame's next section: `else`, `and` or `option`.
    Divider {
        kind: &'static str,
        label: Text,
    },
    End,
    /// `activate`, `deactivate`, or `+`/`-` on an arrow: the lifeline is heavy while active.
    Activation {
        who: Who,
        on: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub from: Who,
    pub to: Who,
    pub arrow: Arrow,
    pub label: Vec<Text>,
    /// Its `autonumber`.
    pub number: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrow {
    pub dotted: bool,
    pub head: Head,
    /// Heads at both ends (`<<->>`).
    pub both: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    /// `->`, `-->`: a line without a head.
    None,
    /// `->>`, `-->>`.
    Filled,
    /// `-)`, `--)`: asynchronous.
    Open,
    /// `-x`, `--x`.
    Cross,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteAt {
    LeftOf(Who),
    RightOf(Who),
    Over(Who, Who),
}

/// A block `end` closes.
enum Open {
    Frame,
    /// `box` and `rect`, which only color and group.
    Group,
}

#[derive(Default)]
struct Parser {
    src: Source,
    seq: Sequence,
    open: Vec<Open>,
    /// The next `autonumber` and its step, while numbering is on.
    numbering: Option<(u32, u32)>,
    /// Inside a multi-line `accDescr { … }`.
    in_description: bool,
}

/// Parse the statements after the `sequenceDiagram` line.
pub(super) fn parse<'a>(
    lines: impl Iterator<Item = (u32, &'a str)>,
    src: Source,
) -> Option<Sequence> {
    let mut p = Parser {
        src,
        ..Parser::default()
    };
    for (n, (_, line)) in lines.enumerate() {
        if n >= MAX_STATEMENTS {
            return None;
        }
        p.statement(line)?;
    }
    // Mermaid wants every frame closed; ada closes them for it.
    for open in p.open.drain(..) {
        if let Open::Frame = open {
            p.seq.events.push(Event::End);
        }
    }
    (!p.seq.participants.is_empty()).then_some(p.seq)
}

impl Parser {
    fn statement(&mut self, line: &str) -> Option<()> {
        if self.in_description {
            self.in_description = !line.contains('}');
            return Some(());
        }
        let word = line
            .split(|c: char| c.is_whitespace() || c == ':')
            .next()
            .unwrap_or("");
        let rest = line[word.len()..].trim();
        match word {
            "participant" | "actor" => self.declare(rest, word == "actor"),
            "create" => {
                let kind = rest.split_whitespace().next().unwrap_or("");
                let rest = rest[kind.len()..].trim();
                match kind {
                    "participant" | "actor" => self.declare(rest, kind == "actor"),
                    _ => None,
                }
            }
            "destroy" | "links" | "link" | "properties" | "details" | "accTitle" => Some(()),
            "accDescr" => {
                self.in_description = rest.starts_with('{') && !rest.contains('}');
                Some(())
            }
            "autonumber" => self.autonumber(rest),
            "title" => {
                let text = rest.strip_prefix(':').unwrap_or(rest).trim();
                self.seq.title = self.src.label(text);
                Some(())
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" => {
                let kind = frame_kind(word)?;
                self.open.push(Open::Frame);
                self.seq.events.push(Event::Begin {
                    kind,
                    label: self.src.one_line(rest),
                });
                Some(())
            }
            "rect" | "box" => {
                self.open.push(Open::Group);
                Some(())
            }
            "else" | "and" | "option" => {
                matches!(self.open.last(), Some(Open::Frame)).then_some(())?;
                self.seq.events.push(Event::Divider {
                    kind: frame_kind(word)?,
                    label: self.src.one_line(rest),
                });
                Some(())
            }
            "end" => {
                if let Open::Frame = self.open.pop()? {
                    self.seq.events.push(Event::End);
                }
                Some(())
            }
            "activate" | "deactivate" => {
                let who = self.who(rest)?;
                self.seq.events.push(Event::Activation {
                    who,
                    on: word == "activate",
                });
                Some(())
            }
            w if w.eq_ignore_ascii_case("note") => self.note(rest),
            _ => self.message(line),
        }
    }

    /// `participant A`, `participant A as Alice`.
    fn declare(&mut self, rest: &str, actor: bool) -> Option<()> {
        let (id, label) = match rest.split_once(" as ") {
            Some((id, label)) => (id, Some(label)),
            None => (rest, None),
        };
        // newer Mermaid takes a `@{…}` shape after the id
        let id = id.split_once("@{").map_or(id, |(id, _)| id);
        let who = self.who(id)?;
        let p = &mut self.seq.participants[usize::from(who)];
        p.actor = actor;
        if let Some(label) = label {
            p.label = self.src.label(label.trim());
        }
        Some(())
    }

    /// The participant `id` names, added on first mention.
    fn who(&mut self, id: &str) -> Option<Who> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        let ps = &mut self.seq.participants;
        if let Some(i) = ps.iter().position(|p| p.id == id) {
            return Some(i as Who);
        }
        if ps.len() >= MAX_PARTICIPANTS {
            return None;
        }
        ps.push(Participant {
            id: id.to_owned(),
            label: self.src.label(id),
            actor: false,
        });
        Some((ps.len() - 1) as Who)
    }

    /// `autonumber`, `autonumber 10`, `autonumber 10 5`, `autonumber off`.
    fn autonumber(&mut self, rest: &str) -> Option<()> {
        let mut words = rest.split_whitespace();
        self.numbering = match words.next() {
            None => Some((1, 1)),
            Some("off") => None,
            Some(start) => {
                let step = words.next().map_or(Some(1), |s| s.parse().ok())?;
                Some((start.parse().ok()?, step))
            }
        };
        Some(())
    }

    /// `left of A: …`, `right of A: …`, `over A: …`, `over A,B: …`.
    fn note(&mut self, rest: &str) -> Option<()> {
        let (place, text) = rest.split_once(':').unwrap_or((rest, ""));
        let place = place.trim();
        let lower = place.to_ascii_lowercase();
        let after = |prefix: &str| {
            lower
                .starts_with(prefix)
                .then(|| place[prefix.len()..].trim())
        };
        let at = if let Some(id) = after("left of") {
            NoteAt::LeftOf(self.who(id)?)
        } else if let Some(id) = after("right of") {
            NoteAt::RightOf(self.who(id)?)
        } else {
            let ids = after("over")?;
            match ids.split_once(',') {
                Some((a, b)) => {
                    let (a, b) = (self.who(a)?, self.who(b)?);
                    NoteAt::Over(a.min(b), a.max(b))
                }
                None => {
                    let a = self.who(ids)?;
                    NoteAt::Over(a, a)
                }
            }
        };
        self.seq.events.push(Event::Note {
            at,
            text: self.src.label(text.trim()),
        });
        Some(())
    }

    /// `A ->> B: text`, with `+`/`-` after the arrow for activations.
    fn message(&mut self, line: &str) -> Option<()> {
        let parsed = message().parse(line);
        let (from, arrow, activation, to) = parsed.result?;
        let text = parsed.state.strip_prefix(':').unwrap_or(parsed.state);
        let (from, to) = (self.who(from)?, self.who(to)?);
        let number = self.numbering.map(|(n, _)| n);
        if let Some((n, step)) = &mut self.numbering {
            *n = n.saturating_add(*step);
        }
        self.seq.events.push(Event::Message(Message {
            from,
            to,
            arrow,
            label: self.src.label(text.trim()),
            number,
        }));
        // `+` activates the receiver; `-` ends the sender's activation, as in Mermaid
        if let Some(on) = activation {
            let who = if on { to } else { from };
            self.seq.events.push(Event::Activation { who, on });
        }
        Some(())
    }
}

fn frame_kind(word: &str) -> Option<&'static str> {
    const KINDS: [&str; 9] = [
        "loop", "alt", "opt", "par", "critical", "break", "else", "and", "option",
    ];
    KINDS.into_iter().find(|k| *k == word)
}

/// A participant's id in a message: anything up to an arrow, `:` or `,`.
fn name<'a>() -> impl StrParser<'a, &'a str> {
    not_empty(item_while(|c: char| {
        !matches!(c, '-' | '<' | '>' | '+' | ':' | ',' | ';')
    }))
}

fn arrow<'a>() -> impl StrParser<'a, Arrow> {
    let a = |dotted: bool, head: Head, both: bool| Arrow { dotted, head, both };
    or!(
        map(skip("<<-->>"), move |_| a(true, Head::Filled, true)),
        map(skip("<<->>"), move |_| a(false, Head::Filled, true)),
        map(skip("-->>"), move |_| a(true, Head::Filled, false)),
        map(skip("--x"), move |_| a(true, Head::Cross, false)),
        map(skip("--)"), move |_| a(true, Head::Open, false)),
        map(skip("-->"), move |_| a(true, Head::None, false)),
        map(skip("->>"), move |_| a(false, Head::Filled, false)),
        map(skip("-x"), move |_| a(false, Head::Cross, false)),
        map(skip("-)"), move |_| a(false, Head::Open, false)),
        map(skip("->"), move |_| a(false, Head::None, false)),
    )
}

/// `from`, the arrow, an activation (`+` on, `-` off) and `to`; the input left is `: text`.
fn message<'a>() -> impl StrParser<'a, (&'a str, Arrow, Option<bool>, &'a str)> {
    let activation = succeed(or!(map(skip('+'), |_| true), map(skip('-'), |_| false)));
    tuplify!(name(), arrow(), activation, name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::{Diagram, parse as parse_diagram};

    fn seq(src: &str) -> Sequence {
        match parse_diagram(src) {
            Some(Diagram::Sequence(s)) => s,
            _ => panic!("didn't parse: {src}"),
        }
    }

    fn message(s: &Sequence, i: usize) -> &Message {
        match &s.events[i] {
            Event::Message(m) => m,
            e => panic!("not a message: {e:?}"),
        }
    }

    #[test]
    fn participants_in_order_of_mention() {
        let s = seq("sequenceDiagram\nparticipant B as Bob<br/>Smith\nA->>B: hi\nactor C");
        let ids: Vec<_> = s.participants.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["B", "A", "C"]);
        assert_eq!(s.participants[0].label, ["Bob", "Smith"]);
        assert!(s.participants[2].actor);
    }

    #[test]
    fn arrows() {
        let src = "sequenceDiagram\nA->B: a\nA-->B: b\nA->>B: c\nA-->>B: d\nA-xB: e\nA--xB: f\n\
                   A-)B: g\nA--)B: h\nA<<->>B: i\nA<<-->>B: j";
        let s = seq(src);
        let got: Vec<_> = (0..10)
            .map(|i| {
                let a = message(&s, i).arrow;
                (a.dotted, a.head, a.both)
            })
            .collect();
        use Head::*;
        assert_eq!(
            got,
            [
                (false, None, false),
                (true, None, false),
                (false, Filled, false),
                (true, Filled, false),
                (false, Cross, false),
                (true, Cross, false),
                (false, Open, false),
                (true, Open, false),
                (false, Filled, true),
                (true, Filled, true),
            ]
        );
    }

    #[test]
    fn messages_with_spaces_lists_and_activations() {
        let s = seq(
            "sequenceDiagram\n  Kamino ->>+ Endor: List<WrappingKey>, (TTL)\n  Endor-->>-Kamino:",
        );
        let m = message(&s, 0);
        assert_eq!((m.from, m.to), (0, 1));
        assert_eq!(m.label, ["List<WrappingKey>, (TTL)"]);
        assert_eq!(s.events[1], Event::Activation { who: 1, on: true });
        assert_eq!(message(&s, 2).label, [""]);
        assert_eq!(s.events[3], Event::Activation { who: 1, on: false });
    }

    #[test]
    fn autonumber_counts_messages() {
        let s =
            seq("sequenceDiagram\nautonumber 10 5\nA->>B: x\nB->>A: y\nautonumber off\nA->>B: z");
        let numbers: Vec<_> = (0..3).map(|i| message(&s, i).number).collect();
        assert_eq!(numbers, [Some(10), Some(15), None]);
    }

    #[test]
    fn notes_frames_and_groups() {
        let src = "sequenceDiagram\nbox Aqua Group\nparticipant A\nend\nNote over B,A: both\n\
                   loop every minute\nA->>B: ping\nalt ok\nB-->>A: pong\nelse failed\nB--xA: no\nend\nend\n\
                   note left of A: l\nNOTE right of B: r";
        let s = seq(src);
        assert_eq!(
            s.events[0],
            Event::Note {
                at: NoteAt::Over(0, 1),
                text: vec!["both".into()]
            }
        );
        assert_eq!(
            s.events[1],
            Event::Begin {
                kind: "loop",
                label: "every minute".into()
            }
        );
        assert!(matches!(s.events[3], Event::Begin { kind: "alt", .. }));
        assert!(matches!(s.events[5], Event::Divider { kind: "else", .. }));
        assert_eq!(s.events[7..9], [Event::End, Event::End]);
        assert!(matches!(
            s.events[9],
            Event::Note {
                at: NoteAt::LeftOf(0),
                ..
            }
        ));
        assert!(matches!(
            s.events[10],
            Event::Note {
                at: NoteAt::RightOf(1),
                ..
            }
        ));
    }

    #[test]
    fn unclosed_frames_close_and_stray_ends_fail() {
        let s = seq("sequenceDiagram\nloop x\nA->>B: y");
        assert_eq!(s.events.last(), Some(&Event::End));
        assert!(parse_diagram("sequenceDiagram\nA->>B: y\nend").is_none());
        assert!(parse_diagram("sequenceDiagram\nelse x").is_none());
    }

    #[test]
    fn what_doesnt_parse_is_left_as_source() {
        assert!(parse_diagram("sequenceDiagram\nA B C").is_none());
        assert!(parse_diagram("sequenceDiagram\n->>B: x").is_none());
        assert!(
            parse_diagram("sequenceDiagram").is_none(),
            "no participants"
        );
        let many: String = (0..70).map(|i| format!("P{i}->>Q: x\n")).collect();
        assert!(parse_diagram(&format!("sequenceDiagram\n{many}")).is_none());
    }

    #[test]
    fn titles_and_descriptions() {
        let s = seq("sequenceDiagram\ntitle: Key flow\naccDescr {\nmany\nlines }\nA->>B: x");
        assert_eq!(s.title, ["Key flow"]);
        assert_eq!(s.events.len(), 1);
    }
}
