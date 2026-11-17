//! past_chats: this project's earlier conversations, for the model to look
//! at when a question is about them ("what did I ask first?"). Chats are
//! saved per project for `/resume` and `rak -c`; without this tool the model
//! in a new chat cannot see them. Read-only, this project's chats only, and
//! nothing reaches the model unless it asks: one tool description is the
//! whole cost.

use super::{Tool, ToolResult};
use crate::sessions::{self, Saved};
use crate::text::{message_text, one_line};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The most text one call hands back.
const ROOM: usize = 12_000;
/// What is shown of an answer; a request is shown longer.
const ANSWER: usize = 600;
const REQUEST: usize = 3_000;

pub struct PastChats {
    /// ~/.config/rak/sessions
    pub dir: PathBuf,
    /// This project's folder in it.
    pub key: String,
}

fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}… (cut, {} characters in all)", &text[..at], text.chars().count()),
        None => text.to_string(),
    }
}

/// Pure — unit-tested. The earlier chats, newest first, numbered from 1.
pub fn listing(chats: &[Saved], now: u64) -> String {
    if chats.is_empty() {
        return "No earlier chats are saved for this project.".to_string();
    }
    let mut out = format!(
        "{} earlier chat{} in this project, newest first (rak keeps the last {}). Read one with `chat`.\n",
        chats.len(),
        if chats.len() == 1 { "" } else { "s" },
        sessions::KEEP
    );
    for (i, chat) in chats.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} · {} messages · first request: {}\n",
            i + 1,
            sessions::age(chat.saved_at, now),
            chat.messages,
            chat.title
        ));
    }
    out
}

/// Pure — unit-tested. One message as a line or two: who, and what they said.
/// Tool results are left out (they are long, and the answer says what came of
/// them); a tool call is its name.
fn shown(message: &Value) -> Option<String> {
    let text = message_text(&message["content"]).unwrap_or_default();
    match message["role"].as_str()? {
        "user" if !text.trim().is_empty() => Some(format!("USER: {}", cut(&text, REQUEST))),
        "assistant" => {
            let calls: Vec<&str> =
                message["tool_calls"].as_array().into_iter().flatten().filter_map(|c| c["function"]["name"].as_str()).collect();
            let mut line = String::new();
            if !text.trim().is_empty() {
                line.push_str(&format!("RAK: {}", cut(&text, ANSWER)));
            }
            if !calls.is_empty() {
                line.push_str(&format!("{}RAK called: {}", if line.is_empty() { "" } else { "\n" }, calls.join(", ")));
            }
            (!line.is_empty()).then_some(line)
        }
        _ => None,
    }
}

/// Pure — unit-tested. A chat's requests and answers from message `from`
/// (counted from 1), as much as fits; says where to go on.
pub fn reading(messages: &[Value], from: usize) -> String {
    let mut out = String::new();
    for (i, message) in messages.iter().enumerate().skip(from.saturating_sub(1)) {
        let Some(line) = shown(message) else { continue };
        if !out.is_empty() && out.len() + line.len() > ROOM {
            out.push_str(&format!("\n(more: {} messages in all; go on with from={})", messages.len(), i + 1));
            return out;
        }
        out.push_str(&format!("[{}] {line}\n", i + 1));
    }
    if out.is_empty() {
        out = format!("Nothing to show from message {from}: the chat has {} messages.", messages.len());
    }
    out
}

/// Pure — unit-tested. The messages of one chat that hold `needle`, any case.
pub fn matches(messages: &[Value], needle: &str) -> Vec<String> {
    let needle = needle.to_lowercase();
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m["role"].as_str(), Some("user" | "assistant")))
        .filter_map(|(i, m)| {
            let text = message_text(&m["content"])?;
            let lower = text.to_lowercase();
            let at = lower.find(&needle)?;
            // Some of the line around it; lowercasing can move byte offsets, so cut on characters.
            let before = lower[..at].chars().count();
            let around: String = text.chars().skip(before.saturating_sub(80)).take(240).collect();
            let who = if m["role"] == "user" { "USER" } else { "RAK" };
            Some(format!("[{}] {who}: {}", i + 1, one_line(&around, 240)))
        })
        .collect()
}

impl PastChats {
    /// Every saved chat of this project but the one being written now.
    fn chats(&self) -> Vec<Saved> {
        let current = sessions::current();
        sessions::list(&self.dir, &self.key).into_iter().filter(|c| sessions::id_of(&c.path) != current).collect()
    }
}

impl Tool for PastChats {
    fn name(&self) -> &str {
        "past_chats"
    }

    fn description(&self) -> &str {
        "Earlier conversations with the user in this project (not this one). No arguments: list them. `chat`: read \
         that one's requests and answers. `search`: find text in all of them. Use it when asked about earlier work or requests."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "chat": { "type": "integer", "description": "Number from the list (1 is the newest)." },
                "from": { "type": "integer", "description": "Message to start reading at." },
                "search": { "type": "string", "description": "Text to find." }
            }
        })
    }

    fn run(&self, args: &Value) -> ToolResult {
        let chats = self.chats();
        if let Some(needle) = args["search"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            let mut out = String::new();
            for (i, chat) in chats.iter().enumerate() {
                let Ok(messages) = sessions::load(&chat.path) else { continue };
                let found = matches(&messages, needle);
                if found.is_empty() {
                    continue;
                }
                out.push_str(&format!("Chat {} ({}):\n", i + 1, chat.title));
                for line in found {
                    if out.len() + line.len() > ROOM {
                        out.push_str("(more matches not shown; search for something narrower)\n");
                        return ToolResult::ok(out);
                    }
                    out.push_str(&format!("  {line}\n"));
                }
            }
            return ToolResult::ok(if out.is_empty() {
                format!("`{needle}` is in none of the {} earlier chats.", chats.len())
            } else {
                out
            });
        }
        match args["chat"].as_u64() {
            None => ToolResult::ok(listing(&chats, sessions::now_seconds())),
            Some(n) => match chats.get((n as usize).wrapping_sub(1)) {
                Some(chat) => match sessions::load(&chat.path) {
                    Ok(messages) => ToolResult::ok(reading(&messages, args["from"].as_u64().unwrap_or(1) as usize)),
                    Err(why) => ToolResult::err(why),
                },
                None => ToolResult::err(format!("There is no chat {n}: the list has {}.", chats.len())),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat() -> Vec<Value> {
        vec![
            json!({ "role": "user", "content": "Build a todo app in Flask with SQLite" }),
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "write_file", "arguments": "{}" } }] }),
            json!({ "role": "tool", "content": "wrote app.py (2000 bytes)" }),
            json!({ "role": "assistant", "content": "The app is in app.py." }),
            json!({ "role": "user", "content": "Add a DARK mode" }),
        ]
    }

    #[test]
    fn a_chat_is_read_as_requests_and_answers_without_tool_output() {
        let text = reading(&chat(), 1);
        assert!(text.starts_with("[1] USER: Build a todo app in Flask with SQLite\n"), "{text}");
        assert!(text.contains("[2] RAK called: write_file\n") && text.contains("[4] RAK: The app is in app.py."), "{text}");
        assert!(!text.contains("wrote app.py"), "a tool's output is not handed back:\n{text}");
        assert!(reading(&chat(), 5).starts_with("[5] USER: Add a DARK mode"));
        assert!(reading(&chat(), 9).contains("has 5 messages"));
    }

    #[test]
    fn a_long_chat_stops_where_the_room_ends_and_says_where_to_go_on() {
        let long: Vec<Value> = (0..40).map(|i| json!({ "role": "user", "content": format!("{i} {}", "word ".repeat(200)) })).collect();
        let text = reading(&long, 1);
        assert!(text.len() < ROOM + 200, "{}", text.len());
        let next: usize = text.rsplit("from=").next().unwrap().trim_end_matches(')').parse().unwrap();
        assert!(next > 1 && next <= 40);
        assert!(reading(&long, next).starts_with(&format!("[{next}] USER: {} word", next - 1)), "nothing skipped, nothing twice");
        let answer = json!({ "role": "assistant", "content": "é".repeat(2000) });
        assert!(shown(&answer).unwrap().contains("(cut, 2000 characters in all)"), "cut on characters, not bytes");
    }

    #[test]
    fn search_finds_text_in_any_case_and_says_who_wrote_it() {
        let found = matches(&chat(), "dark mode");
        assert_eq!(found, ["[5] USER: Add a DARK mode"]);
        assert_eq!(matches(&chat(), "app.py").len(), 1, "requests and answers only, not tool output");
        assert!(matches(&chat(), "postgres").is_empty());
    }

    #[test]
    fn the_list_is_numbered_newest_first_and_says_when_it_is_empty() {
        let saved = |title: &str, at: u64| Saved { path: PathBuf::from("x.json"), title: title.into(), saved_at: at, messages: 4 };
        let text = listing(&[saved("Add a dark mode", 10_000), saved("Build a todo app", 1_000)], 10_060);
        assert!(text.starts_with("2 earlier chats in this project, newest first"), "{text}");
        assert!(text.contains("1. 1 minute ago · 4 messages · first request: Add a dark mode\n"), "{text}");
        assert!(text.contains("2. 2 hours ago · 4 messages · first request: Build a todo app\n"), "{text}");
        assert!(listing(&[], 0).starts_with("No earlier chats"));
    }
}
