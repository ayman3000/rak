//! delegate: hands a self-contained question to a helper agent with a fresh
//! context and read-only tools, and returns only its answer. Searching and
//! reading a large codebase then costs the main conversation one answer
//! instead of every file read on the way.
//!
//! From SwiftAgentKit's `delegate_task`, made smaller: one helper at a time,
//! read-only (so it needs no approvals and two agents never edit one file),
//! and no delegating further. Whoever builds the tool decides how a helper is
//! made (model, tools, how its steps are shown).

use crate::agent::{Agent, Outcome};
use crate::provider::Usage;
use crate::tools::{str_arg, Tool, ToolResult};
use serde_json::{json, Value};
use std::cell::Cell;
use std::rc::Rc;

/// Added to a helper's system prompt.
pub const HELPER_NOTE: &str = "\n\nYou are a helper: another agent gave you one task and sees only your final reply. \
     You cannot see its conversation and cannot change files. Do the task with your tools, then reply with a complete, \
     self-contained answer: the facts, file paths with line numbers, and short quotes of the code that matters. Say what \
     you could not find.";

pub struct Delegate {
    make: Box<dyn Fn() -> Agent>,
    /// What the helpers spent, for the caller to add to its own count.
    pub usage: Rc<Cell<Usage>>,
}

impl Delegate {
    pub fn new(make: Box<dyn Fn() -> Agent>) -> Self {
        Delegate { make, usage: Rc::new(Cell::new(Usage::default())) }
    }
}

impl Tool for Delegate {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "Give a self-contained question to a helper agent with a fresh context and read-only tools; you get back only \
         its answer. Use it to search or understand a large codebase without filling your own context. It sees none \
         of this conversation: put everything it needs in `task`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "task": { "type": "string", "description": "The whole question, with the context it needs." } },
            "required": ["task"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(task) = str_arg(args, "task").map(str::trim).filter(|t| !t.is_empty()) else {
            return ToolResult::err("delegate requires `task`.");
        };
        let mut helper = (self.make)();
        let result = helper.run(task);
        let mut spent = self.usage.get();
        spent.add(helper.usage);
        self.usage.set(spent);
        let footer = format!(
            "[helper: {} turn{}, {} tool call{}]",
            result.turns,
            if result.turns == 1 { "" } else { "s" },
            helper.tool_calls,
            if helper.tool_calls == 1 { "" } else { "s" }
        );
        match result.outcome {
            Outcome::Done => ToolResult::ok(format!("{}\n{footer}", result.answer.trim())),
            Outcome::Stopped => ToolResult::err("The helper was stopped (Ctrl-C)."),
            Outcome::Failed(why) => ToolResult::err(format!("The helper failed: {why}")),
            other => {
                // Out of turns or looping: whatever it last said is still worth having.
                let said = helper.history().iter().rev().find_map(|m| {
                    (m["role"] == "assistant").then(|| m["content"].as_str().unwrap_or("").trim().to_string()).filter(|t| !t.is_empty())
                });
                ToolResult::err(format!(
                    "The helper did not finish ({other:?}). Its last words: {}\n{footer}",
                    said.unwrap_or_else(|| "(nothing)".into())
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Config;
    use crate::provider::{Model, ModelError, Reply};

    struct Says(&'static str);
    impl Model for Says {
        fn chat(&mut self, _: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            Ok(Reply { content: self.0.to_string(), usage: Usage { input: 7, output: 3, ..Default::default() }, ..Default::default() })
        }
    }

    #[test]
    fn the_helper_answers_and_its_tokens_are_counted() {
        let tool = Delegate::new(Box::new(|| {
            let config = Config { system_prompt: format!("helper{HELPER_NOTE}"), max_turns: 5, context_window: 10_000 };
            Agent::new(Box::new(Says("Found it in src/a.rs:12.")), Vec::new(), config)
        }));
        assert!(!tool.needs_approval());
        let answer = tool.run(&json!({ "task": "Where is the login check?" }));
        assert!(answer.ok && answer.text.starts_with("Found it in src/a.rs:12.\n[helper: 1 turn, 0 tool calls]"), "{}", answer.text);
        tool.run(&json!({ "task": "again" }));
        assert_eq!((tool.usage.get().input, tool.usage.get().output), (14, 6));
        assert!(!tool.run(&json!({})).ok);
    }
}
