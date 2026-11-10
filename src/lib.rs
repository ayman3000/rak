//! Rust Agent Kit: the agent loop, an OpenAI-compatible model client and the
//! core coding tools. The `rak` binary (main.rs) is a thin terminal front end.
//!
//! The prompts, tool descriptions and tool behaviour follow sak / SwiftAgentKit
//! closely: sak's low token use comes from those, not from its language.

pub mod agent;
pub mod provider;
pub mod read_slice;
pub mod tools;
