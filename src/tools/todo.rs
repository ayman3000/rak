//! todo: the agent's task list for work with several steps. The model keeps
//! the plan here instead of in its head, the person sees it update, and when
//! the model says it is done with steps still open, rak asks it once to finish
//! them or say why not (`open_note`, used by the agent's finish check).

use super::{Tool, ToolResult};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub text: String,
    /// "pending", "in_progress" or "done".
    pub status: String,
}

#[derive(Default)]
struct State {
    steps: Vec<Step>,
    /// Updated during the current request.
    touched: bool,
}

/// The list, shared by the tool and whoever checks it.
#[derive(Clone, Default)]
pub struct TodoList(Arc<Mutex<State>>);

impl TodoList {
    pub fn steps(&self) -> Vec<Step> {
        self.0.lock().unwrap().steps.clone()
    }

    /// A new request starts: the finish check only looks at a list it updates.
    pub fn begin_request(&self) {
        self.0.lock().unwrap().touched = false;
    }

    /// Pure given the state — unit-tested. What to tell a model that finished
    /// with steps open, once per request; None when there is nothing to say.
    pub fn open_note(&self) -> Option<String> {
        let mut state = self.0.lock().unwrap();
        if !state.touched {
            return None;
        }
        state.touched = false; // once
        let open: Vec<String> = state.steps.iter().filter(|s| s.status != "done").map(|s| format!("- {}", s.text)).collect();
        (!open.is_empty()).then(|| {
            format!(
                "[task list] These steps are not marked done:\n{}\nFinish them now, or mark them done in `todo` if they \
                 are, or say plainly which you are leaving and why. Then give your final summary.",
                open.join("\n")
            )
        })
    }
}

/// Pure — unit-tested. The list as the model and the person see it.
pub fn render(steps: &[Step]) -> String {
    let done = steps.iter().filter(|s| s.status == "done").count();
    let lines: Vec<String> = steps
        .iter()
        .map(|s| {
            let mark = match s.status.as_str() {
                "done" => "[x]",
                "in_progress" => "[>]",
                _ => "[ ]",
            };
            format!("{mark} {}", s.text)
        })
        .collect();
    format!("Task list ({done}/{} done):\n{}", steps.len(), lines.join("\n"))
}

pub struct Todo(pub TodoList);

impl Tool for Todo {
    fn name(&self) -> &str {
        "todo"
    }
    fn description(&self) -> &str {
        "Your task list, for work with three or more steps. Send the whole list each time: at the start, and whenever \
         a step starts or is done. One step in_progress at a time. Skip it for small tasks."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "done"] }
                        },
                        "required": ["text", "status"]
                    }
                }
            },
            "required": ["steps"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(items) = args.get("steps").and_then(Value::as_array) else {
            return ToolResult::err("todo requires `steps`: the whole list, each with `text` and `status`.");
        };
        let mut steps = Vec::new();
        for item in items {
            // Models name the field differently now and then; the meaning is clear.
            let text = ["text", "name", "step", "content", "title", "description"]
                .iter()
                .find_map(|key| item.get(*key).and_then(Value::as_str))
                .or_else(|| item.as_str())
                .map(str::trim)
                .unwrap_or("");
            if text.is_empty() {
                continue;
            }
            let status = match item.get("status").and_then(Value::as_str).unwrap_or("pending") {
                "done" | "completed" => "done",
                "in_progress" | "doing" | "active" => "in_progress",
                _ => "pending",
            };
            steps.push(Step { text: text.to_string(), status: status.to_string() });
        }
        if steps.is_empty() {
            return ToolResult::err("The list is empty: send each step with `text` and `status`.");
        }
        let shown = render(&steps);
        let mut state = self.0 .0.lock().unwrap();
        state.steps = steps;
        state.touched = true;
        ToolResult::ok(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_kept_shown_and_checked_once_per_request() {
        let list = TodoList::default();
        let tool = Todo(list.clone());
        assert!(!tool.needs_approval());
        let shown = tool.run(&json!({ "steps": [
            { "text": "Read the code", "status": "completed" },
            { "text": "Fix the bug", "status": "in_progress" },
            { "text": "Run the tests", "status": "pending" },
            { "text": "  ", "status": "pending" }
        ] }));
        assert!(shown.ok);
        assert_eq!(shown.text, "Task list (1/3 done):\n[x] Read the code\n[>] Fix the bug\n[ ] Run the tests");
        assert_eq!(list.steps().len(), 3);

        let note = list.open_note().unwrap();
        assert!(note.contains("- Fix the bug\n- Run the tests") && !note.contains("Read the code"), "{note}");
        assert!(list.open_note().is_none(), "only once");

        // A new request that never touches the list is not checked.
        list.begin_request();
        assert!(list.open_note().is_none());
        // All done: nothing to say.
        tool.run(&json!({ "steps": [{ "text": "Fix the bug", "status": "done" }] }));
        assert!(list.open_note().is_none());
        assert!(!tool.run(&json!({ "steps": [] })).ok);
        // Other names for the text, and plain strings, are taken as they are meant.
        let other = tool.run(&json!({ "steps": [{ "name": "Explore", "status": "not_started" }, "Write tests"] }));
        assert_eq!(other.text, "Task list (0/2 done):\n[ ] Explore\n[ ] Write tests");
    }
}
