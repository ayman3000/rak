//! Rust Agent Kit: the agent loop, an OpenAI-compatible model client and the
//! core coding tools. The `rak` binary (main.rs and src/cli) is the terminal
//! front end: the chat, its line editor and "/" menu, and Markdown display.
//!
//! The prompts, tool descriptions and tool behaviour follow sak / SwiftAgentKit
//! closely: sak's low token use comes from those, not from its language.

pub mod agent;
pub mod anthropic;
pub mod compact;
pub mod delegate;
pub mod interrupt;
pub mod mcp;
pub mod provider;
pub mod read_slice;
pub mod sessions;
pub mod skills;
pub mod text;
pub mod tools;
