//! The chat's slash commands: what they are called and what they say they do.
//! main.rs runs them.

/// (usage, what it does) — the /help list and the "/" menu, in this order.
pub const HELP: &[(&str, &str)] = &[
    ("/model [name]", "show or switch the model (no name: pick from a list)"),
    ("/models", "list the models the server offers"),
    ("/tools", "list the tools the agent can use"),
    ("/notes", "show the project notes rak loaded (AGENTS.md)"),
    ("/yes", "toggle auto-approve for edits and shell commands"),
    ("/usage", "tokens and tool calls so far"),
    ("/clear", "start a fresh conversation"),
    ("/help", "this list"),
    ("/exit", "quit (or Ctrl-D)"),
];

/// Every name the chat accepts, aliases included — what the "/" menu must
/// leave alone when Enter is pressed.
pub const KNOWN: &[&str] = &[
    "/help", "/?", "/model", "/m", "/models", "/tools", "/notes", "/yes", "/auto", "/usage", "/cost", "/clear", "/reset",
    "/new", "/exit", "/quit", "/q",
];
