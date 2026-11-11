//! The `rak` command's terminal side. The library knows nothing of it.
//!
//!   args         the command line and its help
//!   session      one run: set up from the options, a request run and reported
//!   chat         the chat loop and its slash commands
//!   approval     asking before a tool changes something
//!   output       the answer as it arrives, a line per step
//!   log          --log: every step as JSON lines
//!   mcp_command  rak mcp add|list|remove
//!   keys_command rak keys: the key file and what is set
//!   settings     what is kept between runs
//!   signals      Ctrl-C
//!   editor, menu, markdown, welcome, style, term, commands
//!                the interface's parts, each pure where it can be

pub mod approval;
pub mod args;
pub mod chat;
pub mod commands;
pub mod editor;
pub mod keys_command;
pub mod log;
pub mod markdown;
pub mod mcp_command;
pub mod menu;
pub mod output;
pub mod session;
pub mod settings;
pub mod signals;
pub mod style;
pub mod term;
pub mod welcome;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
