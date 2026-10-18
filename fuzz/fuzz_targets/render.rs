//! Arbitrary bytes → parse → layout at an arbitrary size → paint, with a relayout at a second
//! size, the table and code toggles, and a search (plan P7). Nothing may panic.
//!
//! ```zsh
//! cargo +nightly fuzz run render -- -max_total_time=600
//! ```

#![no_main]

use ada::app::{Computed, Config, Effects, Msg, ScrollBy, Size, State};
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
    let msgs = [
        Msg::Init { size: size(*w, *h) },
        Msg::DocLoaded {
            path: "fuzz.md".into(),
            mtime: None,
            text: text.as_ref().into(),
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
        Msg::SearchOpen,
        Msg::SearchInput {
            pattern: "[a-z]+".into(),
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
