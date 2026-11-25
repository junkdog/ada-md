//! ada: a terminal markdown viewer. The pipeline is Source → [`doc`] → [`layout`] → paint
//! ([`ui`], [`print`](mod@print)); interaction goes through the message architecture in [`app`].

pub mod app;
pub mod cli;
pub mod completions;
pub mod config;
pub mod diagram;
pub mod doc;
pub mod filetype;
pub mod geom;
pub mod git;
pub mod grid;
pub mod guard;
pub mod highlight;
pub mod images;
pub mod iterm;
pub mod kitty;
pub mod layout;
pub mod links;
pub mod pager;
pub mod pixels;
pub mod print;
pub mod probe;
pub mod record;
pub mod search;
pub mod select;
pub mod sixel;
pub mod sniff;
pub mod stdin;
pub mod style;
pub mod theme;
mod toml_walk;
pub mod ui;

/// The cores to work on, eight at most. Read once a process: `available_parallelism` opens and
/// reads the cgroup's CPU quota files, ~10 µs, which was 39% of a small note's print (PRN-007).
pub fn cores() -> usize {
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(8)))
}
