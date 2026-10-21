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
pub mod json;
pub mod kitty;
pub mod layout;
pub mod links;
pub mod logs;
pub mod pixels;
pub mod print;
pub mod probe;
pub mod record;
pub mod search;
pub mod sixel;
pub mod sniff;
pub mod style;
pub mod theme;
mod toml_walk;
pub mod ui;
