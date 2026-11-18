//! The chat's slash commands: what they are called and what they say they do.
//! main.rs runs them.

/// (usage, what it does) — the /help list and the "/" menu, in this order.
pub const HELP: &[(&str, &str)] = &[
    ("/model [name]", "show or switch the model (no name: pick from a list)"),
    ("/models", "list the models the provider offers"),
    ("/provider [name]", "show or switch the provider (no name: list them)"),
    ("/key [provider]", "give or replace an API key (pasted, not shown); no name: pick from a list"),
    ("/effort [level]", "show or switch the reasoning effort (no level: pick from a list)"),
    ("/add-dir [path]", "let the agent read and write in another folder, for this chat"),
    ("/tools", "list the tools the agent can use"),
    ("/mcp", "the MCP servers and their tools"),
    ("/resume", "this folder's saved chats; /resume N picks one up"),
    ("/skills", "the skills the agent can load; /skills add URL adds one, /skills remove NAME deletes one"),
    ("/init", "set up this project's notes in rak/ (or bring them up to date)"),
    ("/notes", "the project's rules and its notes in rak/ (project, roadmap, journal)"),
    ("/yes", "toggle auto-approve for edits and shell commands"),
    ("/usage", "tokens and tool calls so far"),
    ("/jobs", "servers started in the background; /jobs stop N (or all) stops them"),
    ("/clear", "start a fresh conversation"),
    ("/help", "this list"),
    ("/exit", "quit (or Ctrl-D)"),
];

/// Every name the chat accepts, aliases included — what the "/" menu must
/// leave alone when Enter is pressed.
pub const KNOWN: &[&str] = &[
    "/help",
    "/?",
    "/model",
    "/m",
    "/models",
    "/provider",
    "/effort",
    "/key",
    "/keys",
    "/add-dir",
    "/tools",
    "/mcp",
    "/skills",
    "/resume",
    "/load",
    "/init",
    "/notes",
    "/yes",
    "/auto",
    "/usage",
    "/cost",
    "/jobs",
    "/clear",
    "/reset",
    "/new",
    "/exit",
    "/quit",
    "/q",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_command_is_known() {
        for (usage, _) in HELP {
            let name = usage.split(' ').next().unwrap();
            assert!(KNOWN.contains(&name), "{name} is in /help but the chat would not take it");
        }
    }
}
