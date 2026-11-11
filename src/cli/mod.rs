//! The terminal interface of the `rak` command: the welcome panel, the status
//! rule, the line editor with its "/" menu, and Markdown shown as it streams.
//! It follows sak's interface. None of it is part of the library.

pub mod commands;
pub mod editor;
pub mod markdown;
pub mod menu;
pub mod style;
pub mod term;
pub mod welcome;
