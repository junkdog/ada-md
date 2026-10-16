//! ada: a terminal markdown viewer. The pipeline is Source → [`doc`] → [`layout`] → paint
//! ([`ui`], [`print`](mod@print)); interaction goes through the message architecture in [`app`].

pub mod app;
pub mod cli;
pub mod completions;
pub mod config;
pub mod doc;
pub mod git;
pub mod highlight;
pub mod images;
pub mod layout;
pub mod links;
pub mod print;
pub mod record;
pub mod search;
pub mod theme;
mod toml_walk;
pub mod ui;
