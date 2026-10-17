//! A deterministic stand-in for `cargo fuzz` (plan P7) that runs with the other tests: random
//! markdown-ish documents, made of the fragments the parser and layout find hardest, go
//! through parse → layout → paint at random sizes, with the toggles and a search in between.
//! None may panic. The real fuzzer lives in `fuzz/`.

use ada::app::{Computed, Config, Dir, Effects, Msg, ScrollBy, Size, State};
use ada::record::{Entry, replay_headless};
use ada::theme::Theme;

const FRAGMENTS: &[&str] = &[
    "# ",
    "## ",
    "###### ",
    "\n",
    "\n\n",
    "  ",
    "\t",
    "> ",
    "> [!NOTE]\n",
    "- ",
    "* ",
    "1. ",
    "- [ ] ",
    "- [x] ",
    "```",
    "```rust\n",
    "~~~\n",
    "    code\n",
    "|",
    "| a | b |\n",
    "|---|:-:|\n",
    "| --- |",
    "**",
    "*",
    "_",
    "__",
    "~~",
    "`",
    "``",
    "[",
    "]",
    "(",
    ")",
    "[x](b.md)",
    "[[wiki]]",
    "[[a#h|alias]]",
    "![img](i.png)",
    "<https://example.com>",
    "[^1]",
    "[^1]: note\n",
    "<b>",
    "</b>",
    "<!-- c -->",
    "---\n",
    "***",
    "\\",
    "&amp;",
    "&#x1F600;",
    "✅",
    "🠶",
    "日本語",
    "e\u{301}",
    "\u{200b}",
    "\u{fe0f}",
    "wide words that wrap around the column edge ",
    "averyveryverylongwordwithoutanyspacesatallthatmustbreaksomewhere",
];

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn document(rng: &mut Rng) -> String {
    let len = rng.below(200);
    (0..len)
        .map(|_| FRAGMENTS[rng.below(FRAGMENTS.len() as u64) as usize])
        .collect()
}

fn size(rng: &mut Rng) -> Size {
    Size {
        w: 1 + rng.below(160) as u16,
        h: 1 + rng.below(50) as u16,
    }
}

#[test]
fn random_documents_never_panic() {
    let mut rng = Rng(0x00ad_a5ee_d000_0001);
    for _ in 0..400 {
        let text = document(&mut rng);
        let mut msgs = vec![
            Msg::Init {
                size: size(&mut rng),
            },
            Msg::DocLoaded {
                path: "fuzz.md".into(),
                mtime: None,
                text: text.clone().into(),
                doc: Computed(None),
            },
        ];
        for _ in 0..12 {
            msgs.push(match rng.below(10) {
                0 => Msg::Resize {
                    size: size(&mut rng),
                },
                1 => Msg::ToggleTableMode,
                2 => Msg::ToggleCodeWrap,
                3 => Msg::ScrollCodeX { by: 8 },
                4 => Msg::FocusLink { dir: Dir::Next },
                5 => Msg::SearchOpen,
                6 => Msg::SearchInput {
                    pattern: "a|w".into(),
                },
                7 => Msg::SearchSubmit,
                8 => Msg::Scroll {
                    by: ScrollBy::Rows(rng.below(40) as i32 - 20),
                },
                _ => Msg::Scroll {
                    by: ScrollBy::Bottom,
                },
            });
        }
        msgs.push(Msg::Help);
        let entries: Vec<Entry> = msgs
            .into_iter()
            .map(|msg| Entry { at_ms: 0, msg })
            .collect();
        let mut state = State::new(Theme::builtin(), Config::default());
        let frames = replay_headless(&entries, &mut state, &mut Effects::new(None), false);
        assert!(frames.is_ok(), "{text:?}");
    }
}
