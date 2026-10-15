//! preen: a terminal markdown viewer. The pipeline is Source → [`doc`] → [`layout`] → paint
//! ([`ui`], [`print`]); interaction goes through the message architecture in [`app`].

pub mod app;
pub mod cli;
pub mod doc;
pub mod git;
pub mod highlight;
pub mod layout;
pub mod links;
pub mod print;
pub mod record;
pub mod search;
pub mod theme;
pub mod ui;
