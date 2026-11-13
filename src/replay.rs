//! Record a live session's model replies, and play them back with no
//! network. From SwiftAgentKit's `SwiftAgentKitReplay` (`RecordingProvider`,
//! `ReplayProvider`, `Scenario`), in the same JSON shape.
//!
//! A recording turns a session that went wrong into something to run again
//! offline, as often as needed, while fixing the loop around it: the model's
//! side is fixed, so only rak's side changes. Recording is a developer action
//! (`rak --record FILE`, `rak --replay FILE`), never part of a normal run.
//!
//! Only replies are kept, as in SwiftAgentKit's v1: what rak sent is rebuilt
//! by rak itself on replay. A reply's provider-specific form (Anthropic's
//! thinking blocks) is not kept.

use crate::provider::{Model, ModelError, Reply, ToolCall, Usage};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Pure — unit-tested. One reply as a scenario turn.
pub fn turn(reply: &Reply) -> Value {
    json!({
        "text": reply.content,
        "toolCalls": reply.tool_calls.iter().map(|c| json!({ "id": c.id, "name": c.name, "arguments": c.arguments })).collect::<Vec<_>>(),
        "usage": { "promptTokens": reply.usage.input, "completionTokens": reply.usage.output, "cachedTokens": reply.usage.cached }
    })
}

/// Pure — unit-tested. A scenario turn as a reply.
pub fn reply(turn: &Value) -> Reply {
    let tool_calls = turn["toolCalls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, c)| ToolCall {
            id: c["id"].as_str().map(String::from).unwrap_or_else(|| format!("replay-{i}")),
            name: c["name"].as_str().unwrap_or("").to_string(),
            arguments: match &c["arguments"] {
                Value::String(s) => s.clone(),
                Value::Null => "{}".to_string(),
                other => other.to_string(),
            },
        })
        .collect();
    let number = |key: &str| turn["usage"][key].as_u64().unwrap_or(0);
    Reply {
        content: turn["text"].as_str().unwrap_or("").to_string(),
        tool_calls,
        usage: Usage { input: number("promptTokens"), output: number("completionTokens"), cached: number("cachedTokens"), cache_write: 0 },
        native: None,
    }
}

/// Wraps a real model: every reply passes through and is written to `file`
/// as it arrives, so a session that crashes is still recorded.
pub struct Recording {
    inner: Box<dyn Model>,
    file: PathBuf,
    turns: Rc<RefCell<Vec<Value>>>,
}

impl Recording {
    pub fn new(inner: Box<dyn Model>, file: &Path) -> Self {
        Recording { inner, file: file.to_path_buf(), turns: Rc::new(RefCell::new(Vec::new())) }
    }

    fn keep(&self, reply: &Reply) {
        self.turns.borrow_mut().push(turn(reply));
        let name = self.file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let scenario = json!({ "name": name, "turns": *self.turns.borrow() });
        let _ = std::fs::write(&self.file, serde_json::to_string_pretty(&scenario).unwrap_or_default());
    }
}

impl Model for Recording {
    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError> {
        let reply = self.inner.chat(messages, tools)?;
        self.keep(&reply);
        Ok(reply)
    }
    fn chat_stream(&mut self, messages: &[Value], tools: &[Value], on_text: &mut dyn FnMut(&str)) -> Result<Reply, ModelError> {
        let reply = self.inner.chat_stream(messages, tools, on_text)?;
        self.keep(&reply);
        Ok(reply)
    }
    fn list_models(&self) -> Result<Vec<String>, String> {
        self.inner.list_models()
    }
    fn context_window(&self) -> Option<usize> {
        self.inner.context_window()
    }
    fn sees_images(&self) -> Option<bool> {
        self.inner.sees_images()
    }
}

/// Plays a scenario's replies in order, never touching the network. Every
/// request it was sent is kept, for a test to check what rak built.
pub struct Replay {
    turns: Vec<Value>,
    next: usize,
    pub requests: Rc<RefCell<Vec<Vec<Value>>>>,
}

impl Replay {
    pub fn from_file(file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let scenario: Value = serde_json::from_str(&text).map_err(|e| format!("{} is not a scenario: {e}", file.display()))?;
        Self::from_scenario(&scenario).ok_or_else(|| format!("{} has no `turns`", file.display()))
    }

    pub fn from_scenario(scenario: &Value) -> Option<Self> {
        let turns = scenario["turns"].as_array()?.clone();
        Some(Replay { turns, next: 0, requests: Rc::new(RefCell::new(Vec::new())) })
    }
}

impl Model for Replay {
    fn chat(&mut self, messages: &[Value], _tools: &[Value]) -> Result<Reply, ModelError> {
        self.requests.borrow_mut().push(messages.to_vec());
        let Some(turn) = self.turns.get(self.next) else {
            return Err(ModelError::Fatal(format!("the recording ran out after {} replies", self.turns.len())));
        };
        self.next += 1;
        Ok(reply(turn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    struct Fixed(Vec<Reply>);

    impl Model for Fixed {
        fn chat(&mut self, _: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            Ok(self.0.remove(0))
        }
    }

    fn sample() -> Vec<Reply> {
        vec![
            Reply {
                content: "Looking.".into(),
                tool_calls: vec![ToolCall { id: "c1".into(), name: "list_dir".into(), arguments: "{\"path\":\".\"}".into() }],
                usage: Usage { input: 100, output: 5, cached: 80, cache_write: 0 },
                native: None,
            },
            Reply {
                content: "Two files.".into(),
                tool_calls: vec![],
                usage: Usage { input: 140, output: 3, ..Default::default() },
                native: None,
            },
        ]
    }

    #[test]
    fn a_recorded_session_plays_back_the_same_replies() {
        let file = temp_dir("replay").join("bug.json");
        let mut recording = Recording::new(Box::new(Fixed(sample())), &file);
        let mut seen = Vec::new();
        for _ in 0..2 {
            seen.push(recording.chat_stream(&[json!({ "role": "user", "content": "hi" })], &[], &mut |_| {}).unwrap());
        }
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(saved["name"], "bug");
        assert_eq!(saved["turns"][0]["toolCalls"][0]["name"], "list_dir");
        let mut replay = Replay::from_file(&file).unwrap();
        for original in &seen {
            let played = replay.chat(&[json!({ "role": "user", "content": "again" })], &[]).unwrap();
            assert_eq!((played.content.as_str(), played.tool_calls.len()), (original.content.as_str(), original.tool_calls.len()));
            assert_eq!(played.usage.cached, original.usage.cached);
        }
        assert_eq!(replay.tool_calls_args(), "{\"path\":\".\"}");
        assert!(replay.chat(&[], &[]).unwrap_err().to_string_lossy().contains("ran out after 2"));
        assert_eq!(replay.requests.borrow().len(), 3, "every request kept");
    }

    #[test]
    fn a_scenario_written_by_swiftagentkit_reads_too() {
        // SwiftAgentKit encodes arguments as an object and may leave out ids.
        let swift = json!({ "name": "x", "turns": [{ "text": "", "toolCalls": [{ "name": "read_file", "arguments": { "path": "a" } }] }] });
        let r = reply(&Replay::from_scenario(&swift).unwrap().turns[0]);
        assert_eq!((r.tool_calls[0].id.as_str(), r.tool_calls[0].arguments.as_str()), ("replay-0", "{\"path\":\"a\"}"));
    }

    impl Replay {
        fn tool_calls_args(&self) -> String {
            reply(&self.turns[0]).tool_calls[0].arguments.clone()
        }
    }

    trait Lossy {
        fn to_string_lossy(&self) -> String;
    }

    impl Lossy for ModelError {
        fn to_string_lossy(&self) -> String {
            format!("{self:?}")
        }
    }
}
