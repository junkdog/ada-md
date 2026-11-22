//! Arbitrary bytes → parse → layout at an arbitrary size → paint, with a relayout at a second
//! size, the table, code and properties toggles, and a search (plan P7). Nothing may panic.
//! Every note the bytes embed is their own first 128 bytes, transcluded two notes deep (OBS-007).
//!
//! ```zsh
//! cargo +nightly fuzz run render -- -max_total_time=600
//! ```

#![no_main]

use ada::app::{Computed, Config, Cursor, Effects, Msg, ScrollBy, Size, State};
use ada::doc::embed::{self, Note};
use ada::doc::{Embeds, Vault};
use ada::record::{Entry, replay_headless};
use ada::theme::Theme;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [w, h, w2, rest @ ..] = data else {
        return;
    };
    let size = |w: u8, h: u8| Size {
        w: u16::from(w).max(1),
        h: u16::from(h % 64).max(1),
    };
    let text = String::from_utf8_lossy(rest);
    let mut head = text.len().min(128);
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let head: std::sync::Arc<str> = text[..head].into();
    let notes = embed::targets(&text).into_iter().flat_map(|target| {
        [("fuzz.md", "n0.md"), ("n0.md", "n1.md")].map(|(from, path)| Note {
            from: from.into(),
            target: target.clone(),
            path: path.into(),
            mtime: None,
            text: head.clone(),
        })
    });
    let embeds = Embeds(notes.collect::<Vec<_>>().into());
    let msgs = [
        Msg::Init { size: size(*w, *h) },
        Msg::DocLoaded {
            path: "fuzz.md".into(),
            mtime: None,
            text: text.as_ref().into(),
            // `h`'s top bit, which its size leaves, reads the note as in an Obsidian vault
            vault: if h & 0x80 != 0 {
                Vault::Obsidian
            } else {
                Vault::None
            },
            embeds,
            doc: Computed(None),
        },
        Msg::Scroll {
            by: ScrollBy::Rows(i32::from(*h)),
        },
        Msg::Resize {
            size: size(*w2, *h),
        },
        Msg::ToggleTableMode,
        Msg::ToggleCodeWrap,
        Msg::ToggleLineNumbers,
        // a note's properties hidden, then searched with them hidden
        Msg::ToggleProperties,
        Msg::SearchOpen,
        Msg::SearchInput {
            pattern: "[a-z]+".into(),
            cursor: Cursor::END,
        },
        Msg::SearchSubmit,
        Msg::Scroll {
            by: ScrollBy::Bottom,
        },
    ];
    let entries: Vec<Entry> = msgs
        .into_iter()
        .map(|msg| Entry { at_ms: 0, msg })
        .collect();
    let mut state = State::new(Theme::builtin(), Config::default());
    replay_headless(&entries, &mut state, &mut Effects::new(None), false).expect("headless");
});
