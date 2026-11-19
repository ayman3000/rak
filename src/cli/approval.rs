//! Asking before a tool changes something. -y (or /yes) runs everything;
//! otherwise rak asks at the terminal, and with no terminal to ask it stops
//! rather than let the model look for another way.

use super::output::{describe, Ui};
use super::style::{paint, Style};
use rak::agent::Approval;
use rak::interrupt::Stop;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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
    /// Not one of the answers: asked again, never taken as a no or a yes.
    Unclear,
}

/// Pure — unit-tested. `A` (capital) is "everything"; `a` is "always for this
/// tool"; `s` only means something for an MCP server's tool. Enter alone and
/// `n` are a no. Anything else is unclear: a slip of the hand should not end
/// an hour's request, and must not allow anything either.
pub fn answer(typed: &str, has_server: bool) -> Answer {
    match typed.trim() {
        "y" | "Y" | "yes" | "Yes" => Answer::Once,
        "a" | "always" => Answer::AlwaysTool,
        "s" | "server" if has_server => Answer::AlwaysServer,
        "A" | "all" | "everything" => Answer::Everything,
        "" | "n" | "N" | "no" | "No" => Answer::No,
        _ => Answer::Unclear,
    }
}

/// Pure — unit-tested. What was typed, with keys that show nothing on screen
/// (an arrow, a control key) made visible, cut short.
pub fn as_typed(typed: &str) -> String {
    let shown: String = typed.trim_end_matches(['\n', '\r']).chars().flat_map(|c| c.escape_debug()).take(40).collect();
    shown.replace("\\u{1b}", "<Esc>")
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
        // A skill is instructions every later session follows: shown whole.
        ("save_skill", _) => {
            let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").trim();
            let scope = if text("scope") == "project" { "this project only" } else { "every project" };
            let body: Vec<String> = text("instructions").lines().map(|l| format!("    │ {l}")).collect();
            format!("save_skill \"{}\" for {scope}:\n    {}\n{}", text("name"), text("description"), body.join("\n"))
        }
        // A script that will run on this machine whenever the tool is called: shown whole.
        ("save_tool", _) => {
            let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").trim();
            let scope = if text("scope") == "project" { "this project only" } else { "every project" };
            let asks = if args.get("safe") == Some(&Value::Bool(true)) {
                "runs without asking (marked as changing nothing)"
            } else {
                "asks before each run"
            };
            let needs: Vec<&str> = args["needs"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            let keys = if needs.is_empty() { String::new() } else { format!("\n    is given the keys: {}", needs.join(", ")) };
            let tried = match args.get("test_input").filter(|v| v.is_object()) {
                Some(input) => format!("\n    and is run once now with {input}"),
                None => String::new(),
            };
            let body: Vec<String> = text("code").lines().map(|l| format!("    │ {l}")).collect();
            format!(
                "save_tool \"{}\" ({}) for {scope}:\n    {}\n    {asks}{keys}{tried}\n{}",
                text("name"),
                text("language"),
                text("description"),
                body.join("\n")
            )
        }
        // Steps a later session can run in one go: shown whole.
        ("save_routine", _) => {
            let name = args.get("name").and_then(Value::as_str).unwrap_or("");
            let steps: Vec<String> = args["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, s)| format!("    {}. {} {}", i + 1, s["tool"].as_str().unwrap_or("?"), s["arguments"]))
                .collect();
            format!("save_routine \"{name}\":\n{}", steps.join("\n"))
        }
        // A note every later session gets: shown whole.
        ("remember", _) => {
            let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("").trim();
            let scope = if text("scope") == "everywhere" { "every project" } else { "this project" };
            format!("remember, for {scope}:\n    │ {}", text("note"))
        }
        _ => describe(name, args),
    };
    format!("? allow {what}\n  [y]es  [n]o  [a]lways for {name}{scope}  [A] everything this session › ")
}

/// What the person has allowed for the rest of the chat: `A` (everything),
/// `a` (a tool) or `s` (an MCP server's tools). Saved with the chat, so
/// continuing it (`rak -c`, `/resume`) does not ask for the same things again.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Granted {
    pub everything: bool,
    pub tools: std::collections::BTreeSet<String>,
    pub servers: std::collections::BTreeSet<String>,
    /// When the first of these was given (seconds since 1970): a continued
    /// chat keeps them only for `allow_hours` after that.
    pub since: u64,
}

/// How long a continued chat keeps what was allowed, when nothing is set.
pub const DEFAULT_ALLOW_HOURS: u64 = 24;

/// Pure — unit-tested. `RAK_ALLOW_HOURS` as a number of hours: 0 means a
/// continued chat always asks again. Empty or not a number: the default.
pub fn allow_hours(setting: Option<&str>) -> u64 {
    setting.map(str::trim).filter(|typed| !typed.is_empty()).and_then(|typed| typed.parse::<u64>().ok()).unwrap_or(DEFAULT_ALLOW_HOURS)
}

fn hours(n: u64) -> String {
    format!("{n} hour{}", if n == 1 { "" } else { "s" })
}

impl Granted {
    fn holds_anything(&self) -> bool {
        self.everything || !self.tools.is_empty() || !self.servers.is_empty()
    }

    /// Called when the person answers a, s or A: the clock starts at the first.
    pub fn stamp(&mut self, now: u64) {
        if self.since == 0 {
            self.since = now;
        }
    }

    /// Pure — unit-tested. Hours left before a continued chat asks again;
    /// None when the time is up, or `limit` is 0.
    pub fn hours_left(&self, now: u64, limit: u64) -> Option<u64> {
        let end = self.since.saturating_add(limit.saturating_mul(3600));
        (limit > 0 && now < end).then(|| (end - now).div_ceil(3600))
    }

    /// Pure — unit-tested. What a continued chat does with the approvals it
    /// was saved with: keeps them and says so, or drops them and says why.
    /// None: there were none.
    pub fn on_continue(&mut self, now: u64, limit: u64) -> Option<String> {
        if !self.holds_anything() {
            return None;
        }
        match self.hours_left(now, limit) {
            Some(left) => self.notice().map(|said| format!("{said} This lasts {} more.", hours(left))),
            None => {
                *self = Granted::default();
                Some(match limit {
                    0 => "What you allowed in this chat is not kept when it is continued (RAK_ALLOW_HOURS=0): rak asks again.".to_string(),
                    _ => format!("What you allowed in this chat was given more than {} ago: rak asks again.", hours(limit)),
                })
            }
        }
    }

    /// Pure — unit-tested. As it is kept in the saved chat.
    pub fn to_json(&self) -> Value {
        serde_json::json!({ "everything": self.everything, "tools": self.tools, "servers": self.servers, "since": self.since })
    }

    /// Pure — unit-tested. From a saved chat; anything missing or odd is "not allowed".
    pub fn from_json(value: &Value) -> Granted {
        let names = |key: &str| value[key].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string)).collect();
        Granted {
            everything: value["everything"] == true,
            tools: names("tools"),
            servers: names("servers"),
            since: value["since"].as_u64().unwrap_or(0),
        }
    }

    /// Pure — unit-tested. One line said when a chat is continued with
    /// approvals still in force, so nothing runs unasked as a surprise.
    pub fn notice(&self) -> Option<String> {
        let how = "/yes turns asking back on.";
        if self.everything {
            return Some(format!("You allowed everything in this chat (A): edits and commands run without asking. {how}"));
        }
        let mut allowed: Vec<String> = self.tools.iter().cloned().collect();
        allowed.extend(self.servers.iter().map(|server| format!("all of {server}'s tools")));
        (!allowed.is_empty()).then(|| format!("Still allowed without asking in this chat: {}. {how}", allowed.join(", ")))
    }
}

/// What the prompt needs to know.
pub struct Prompter {
    pub ui: Rc<RefCell<Ui>>,
    /// Auto mode: shared with /yes, which turns it on and off.
    pub auto: Rc<Cell<bool>>,
    pub stop: Stop,
    /// MCP tool name → its server.
    /// Shared with the session: servers still connecting when the chat
    /// starts add their tools here later.
    pub server_of: Rc<RefCell<HashMap<String, String>>>,
    /// The "always" answers so far, shared with the session, which saves them with the chat.
    pub granted: Rc<RefCell<Granted>>,
    /// Someone is at a terminal to answer.
    pub interactive: bool,
    pub color: bool,
}

/// The approval step for the agent: remembers the "always" answers for the
/// rest of the session.
pub fn prompter(p: Prompter) -> impl FnMut(&str, &Value) -> Approval {
    move |name, args| {
        let server: Option<String> = p.server_of.borrow().get(name).cloned();
        let allowed = {
            let granted = p.granted.borrow();
            granted.everything || granted.tools.contains(name) || server.as_ref().is_some_and(|s| granted.servers.contains(s))
        };
        if p.auto.get() || allowed {
            return Approval::Yes;
        }
        p.ui.borrow_mut().end_line();
        if !p.interactive {
            eprintln!("  {name} needs approval and there's no terminal to ask — stopping (use -y to allow)");
            return Approval::No;
        }
        // Only what is typed after the question is on screen counts.
        super::term::discard_typed_ahead();
        eprint!("{}", paint(p.color, Style::Yellow, &question(name, args, server.as_deref())));
        let options = if server.is_some() { "y, n, a, s or A" } else { "y, n, a or A" };
        for attempt in 0..3 {
            let _ = std::io::stderr().flush();
            let mut typed = String::new();
            let read = std::io::stdin().lock().read_line(&mut typed);
            if p.stop.requested() {
                return Approval::No;
            }
            match answer(&typed, server.is_some()) {
                Answer::Once => return Approval::Yes,
                Answer::AlwaysTool => {
                    let mut granted = p.granted.borrow_mut();
                    granted.tools.insert(name.to_string());
                    granted.stamp(rak::sessions::now_seconds());
                    return Approval::Yes;
                }
                Answer::AlwaysServer => {
                    let mut granted = p.granted.borrow_mut();
                    granted.servers.extend(server);
                    granted.stamp(rak::sessions::now_seconds());
                    return Approval::Yes;
                }
                Answer::Everything => {
                    let mut granted = p.granted.borrow_mut();
                    granted.everything = true;
                    granted.stamp(rak::sessions::now_seconds());
                    return Approval::Yes;
                }
                Answer::No => return Approval::No,
                // No more input (the terminal closed) is a no, not a loop.
                Answer::Unclear if attempt == 2 || !matches!(read, Ok(n) if n > 0) => return Approval::No,
                Answer::Unclear => {
                    let again = format!("  rak read \"{}\", which is not an answer. Type {options}, then Enter › ", as_typed(&typed));
                    eprint!("{}", paint(p.color, Style::Yellow, &again));
                }
            }
        }
        Approval::No
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
        for typed in ["", "\n", "n", "N", "no", " no \n"] {
            assert_eq!(answer(typed, true), Answer::No, "{typed:?}");
        }
        // Anything else is asked about again: never a yes, and not a no that ends the request.
        for typed in ["maybe", "yy", "Yes please", "ya", "\u{1b}[Aa", "aa", "ش", "s x"] {
            assert_eq!(answer(typed, true), Answer::Unclear, "{typed:?}");
        }
        assert_eq!(answer("s", false), Answer::Unclear, "no server to allow: asked again");
        // Keys that show nothing on screen are made visible in the second question.
        assert_eq!(as_typed("\u{1b}[Aa\n"), "<Esc>[Aa");
        assert_eq!(as_typed("ya\r\n"), "ya");
        assert!(as_typed(&"x".repeat(200)).chars().count() <= 40);
    }

    #[test]
    fn what_was_allowed_is_kept_with_the_chat_and_said_when_it_is_continued() {
        let mut granted = Granted::default();
        assert!(granted.notice().is_none());
        granted.tools.insert("edit_file".into());
        granted.servers.insert("files".into());
        let notice = granted.notice().unwrap();
        assert!(notice.contains("edit_file, all of files's tools") && notice.contains("/yes turns asking back on"), "{notice}");
        assert_eq!(Granted::from_json(&granted.to_json()), granted);
        granted.everything = true;
        assert!(granted.notice().unwrap().starts_with("You allowed everything in this chat (A)"));
        assert_eq!(Granted::from_json(&granted.to_json()), granted);
        // The clock starts at the first "always" answer, and a later one does not restart it.
        let hour = 3600;
        let mut timed = Granted::default();
        timed.stamp(1_000);
        timed.stamp(9_000);
        assert_eq!(timed.since, 1_000);
        timed.everything = true;
        assert_eq!(timed.hours_left(1_000, 24), Some(24));
        assert_eq!(timed.hours_left(1_000 + 23 * hour + 60, 24), Some(1), "under an hour left is said as 1 hour");
        assert_eq!(timed.hours_left(1_000 + 24 * hour, 24), None);
        assert_eq!(timed.hours_left(1_000, 0), None, "0: never kept");
        // Continued in time: kept, and said with how long is left.
        let mut kept = timed.clone();
        let said = kept.on_continue(1_000 + 20 * hour, 24).unwrap();
        assert!(said.starts_with("You allowed everything") && said.ends_with("This lasts 4 hours more."), "{said}");
        assert!(kept.everything);
        // Continued too late: dropped, and said why.
        let mut late = timed.clone();
        let said = late.on_continue(1_000 + 25 * hour, 24).unwrap();
        assert!(said.contains("more than 24 hours ago: rak asks again"), "{said}");
        assert_eq!(late, Granted::default());
        let mut never = timed.clone();
        assert!(never.on_continue(1_000, 0).unwrap().contains("RAK_ALLOW_HOURS=0"));
        assert_eq!(never, Granted::default());
        // One with no time on it (a chat saved by an earlier build) is dropped too; nothing allowed says nothing.
        let mut old = Granted { everything: true, ..Default::default() };
        assert!(old.on_continue(2_000_000_000, 24).unwrap().contains("asks again"));
        assert_eq!(Granted::default().on_continue(1_000, 24), None);
        // The setting: hours, 0 for never, the default when empty or not a number.
        assert_eq!(
            (allow_hours(None), allow_hours(Some("")), allow_hours(Some(" 72 ")), allow_hours(Some("0")), allow_hours(Some("a day"))),
            (24, 24, 72, 0, 24)
        );
        // A chat saved before this, or a damaged entry, allows nothing.
        assert_eq!(Granted::from_json(&Value::Null), Granted::default());
        assert_eq!(Granted::from_json(&json!({ "everything": "yes", "tools": "edit_file" })), Granted::default());
    }

    #[test]
    fn the_question_offers_the_server_only_for_a_servers_tool() {
        let args = json!({ "command": "ls" });
        assert!(!question("run_shell", &args, None).contains("[s]"));
        assert!(question("files__write", &args, Some("files")).contains("[s] all files tools"));
        assert!(question("run_shell", &args, None).starts_with("? allow run_shell:\n    ls\n"));
        let note = json!({ "note": "Prefers tabs.", "scope": "everywhere" });
        assert!(question("remember", &note, None).starts_with("? allow remember, for every project:\n    │ Prefers tabs.\n"));
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

    #[test]
    fn a_tool_is_shown_whole_with_what_it_is_given_before_it_is_saved() {
        let tool = json!({ "name": "search_brave", "description": "Searches the web.", "language": "python",
            "code": "import os\nprint(os.environ['BRAVE_API_KEY'][:0])", "needs": ["BRAVE_API_KEY"], "test_input": { "q": "x" } });
        let asked = question("save_tool", &tool, None);
        assert!(
            asked.starts_with(
                "? allow save_tool \"search_brave\" (python) for every project:\n    Searches the web.\n    asks before each run"
            ),
            "{asked}"
        );
        assert!(asked.contains("is given the keys: BRAVE_API_KEY") && asked.contains("run once now with {\"q\":\"x\"}"), "{asked}");
        assert!(asked.contains("    │ import os\n    │ print(os.environ['BRAVE_API_KEY'][:0])\n  [y]es"), "{asked}");
        let safe = json!({ "name": "wc", "description": "d", "language": "shell", "code": "wc -w", "safe": true, "scope": "project" });
        let asked = question("save_tool", &safe, None);
        assert!(asked.contains("for this project only") && asked.contains("runs without asking (marked as changing nothing)"), "{asked}");
        assert!(!asked.contains("is given the keys") && !asked.contains("run once"), "{asked}");
    }

    #[test]
    fn a_skill_is_shown_whole_before_it_is_saved() {
        let skill = json!({ "name": "Deploy", "description": "Ship to staging", "instructions": "1. Test.\n2. Push.", "scope": "project" });
        let asked = question("save_skill", &skill, None);
        assert!(
            asked.starts_with(
                "? allow save_skill \"Deploy\" for this project only:\n    Ship to staging\n    │ 1. Test.\n    │ 2. Push.\n  [y]es"
            ),
            "{asked}"
        );
    }
}
