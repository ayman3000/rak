//! Asking before a tool changes something. -y (or /yes) runs everything;
//! otherwise rak asks at the terminal, and with no terminal to ask it stops
//! rather than let the model look for another way.

use super::output::{describe, Ui};
use super::style::{paint, Style};
use rak::agent::Approval;
use rak::interrupt::Stop;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::rc::Rc;

/// What the person typed at the prompt. Anything that isn't a yes is a no.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Once,
    /// This tool, for the rest of the session.
    AlwaysTool,
    /// Every tool of this MCP server, for the rest of the session.
    AlwaysServer,
    Everything,
    No,
}

/// Pure — unit-tested. `A` (capital) is "everything"; `a` is "always for this
/// tool"; `s` only means something for an MCP server's tool.
pub fn answer(typed: &str, has_server: bool) -> Answer {
    match typed.trim() {
        "y" | "Y" | "yes" => Answer::Once,
        "a" | "always" => Answer::AlwaysTool,
        "s" | "server" if has_server => Answer::AlwaysServer,
        "A" | "all" | "everything" => Answer::Everything,
        _ => Answer::No,
    }
}

/// Pure — unit-tested. The question, for `name` called with `args`.
/// A shell command is shown whole, line breaks kept: nobody should approve a
/// command they could only read the start of. Other tools get their short
/// description.
pub fn question(name: &str, args: &Value, server: Option<&str>) -> String {
    let scope = server.map(|s| format!("  [s] all {s} tools")).unwrap_or_default();
    let what = match (name, args.get("command").and_then(Value::as_str)) {
        ("run_shell", Some(command)) => {
            let background = if args.get("background") == Some(&Value::Bool(true)) { " (in the background)" } else { "" };
            let lines: Vec<String> = command.trim_end().lines().map(|l| format!("    {l}")).collect();
            format!("run_shell{background}:\n{}", lines.join("\n"))
        }
        _ => describe(name, args),
    };
    format!("? allow {what}\n  [y]es  [n]o  [a]lways for {name}{scope}  [A] everything this session › ")
}

/// What the prompt needs to know.
pub struct Prompter {
    pub ui: Rc<RefCell<Ui>>,
    /// Auto mode: shared with /yes, which turns it on and off.
    pub auto: Rc<Cell<bool>>,
    pub stop: Stop,
    /// MCP tool name → its server.
    pub server_of: HashMap<String, String>,
    /// Someone is at a terminal to answer.
    pub interactive: bool,
    pub color: bool,
}

/// The approval step for the agent: remembers the "always" answers for the
/// rest of the session.
pub fn prompter(p: Prompter) -> impl FnMut(&str, &Value) -> Approval {
    let mut tools: HashSet<String> = HashSet::new();
    let mut servers: HashSet<String> = HashSet::new();
    let mut everything = false;
    move |name, args| {
        let server = p.server_of.get(name);
        if p.auto.get() || everything || tools.contains(name) || server.is_some_and(|s| servers.contains(s)) {
            return Approval::Yes;
        }
        p.ui.borrow_mut().end_line();
        if !p.interactive {
            eprintln!("  {name} needs approval and there's no terminal to ask — stopping (use -y to allow)");
            return Approval::No;
        }
        eprint!("{}", paint(p.color, Style::Yellow, &question(name, args, server.map(String::as_str))));
        let _ = std::io::stderr().flush();
        let mut typed = String::new();
        let _ = std::io::stdin().lock().read_line(&mut typed);
        if p.stop.requested() {
            return Approval::No;
        }
        match answer(&typed, server.is_some()) {
            Answer::Once => {}
            Answer::AlwaysTool => {
                tools.insert(name.to_string());
            }
            Answer::AlwaysServer => servers.extend(server.cloned()),
            Answer::Everything => everything = true,
            Answer::No => return Approval::No,
        }
        Approval::Yes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_are_read_and_anything_else_is_a_no() {
        assert_eq!(answer("y\n", false), Answer::Once);
        assert_eq!(answer(" yes ", false), Answer::Once);
        assert_eq!(answer("a", false), Answer::AlwaysTool);
        assert_eq!(answer("A", false), Answer::Everything);
        assert_eq!(answer("s", true), Answer::AlwaysServer);
        assert_eq!(answer("s", false), Answer::No, "no server to allow");
        for typed in ["", "n", "no", "maybe", "yy", "Yes please"] {
            assert_eq!(answer(typed, true), Answer::No, "{typed:?}");
        }
    }

    #[test]
    fn the_question_offers_the_server_only_for_a_servers_tool() {
        let args = json!({ "command": "ls" });
        assert!(!question("run_shell", &args, None).contains("[s]"));
        assert!(question("files__write", &args, Some("files")).contains("[s] all files tools"));
        assert!(question("run_shell", &args, None).starts_with("? allow run_shell:\n    ls\n"));
    }

    #[test]
    fn a_long_or_multi_line_command_is_shown_whole() {
        let long = format!("echo {}", "x".repeat(300));
        assert!(question("run_shell", &json!({ "command": long }), None).contains(&"x".repeat(300)));
        let script = json!({ "command": "mkdir -p Build\nswiftc App.swift -o Build/App\n", "background": true });
        let asked = question("run_shell", &script, None);
        assert!(
            asked.starts_with("? allow run_shell (in the background):\n    mkdir -p Build\n    swiftc App.swift -o Build/App\n  [y]es"),
            "{asked}"
        );
    }
}
