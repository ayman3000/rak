//! delegate: hands a self-contained question to a helper agent with a fresh
//! context and read-only tools, and returns only its answer. Searching and
//! reading a large codebase then costs the main conversation one answer
//! instead of every file read on the way.
//!
//! From SwiftAgentKit's `delegate_task` and `SubAgentGate`: several
//! `delegate` calls in one reply run at the same time, each helper on its own
//! thread, at most `Host::limit` at once (the rest wait their turn). Helpers
//! are read-only, so they need no approvals and two never edit one file, and
//! they cannot delegate further.
//!
//! Everything that is not thread-safe stays on the calling thread: the
//! `Host` makes each helper's parts, prints its steps and answers its
//! fallback questions; the helper threads only send messages to it.

use crate::agent::{Agent, Config, Event, Outcome};
use crate::fallback::{Failure, Next, Switch};
use crate::interrupt::Stop;
use crate::provider::{Model, Usage};
use crate::tools::{str_arg, Tool, ToolResult};
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

/// Added to a helper's system prompt.
pub const HELPER_NOTE: &str = "\n\nYou are a helper: another agent gave you one task and sees only your final reply. \
     You cannot see its conversation and cannot change files. Do the task with your tools, then reply with a complete, \
     self-contained answer: the facts, file paths with line numbers, and short quotes of the code that matters. Say what \
     you could not find.";

/// What one helper is made of: everything it needs on its own thread.
pub struct Helper {
    pub model: Box<dyn Model + Send>,
    pub tools: Vec<Box<dyn Tool + Send>>,
    pub config: Config,
    pub stop: Stop,
    /// Whether to ask the host for another model when this one fails.
    pub falls_back: bool,
}

/// The model a helper carries on with after a failure.
pub enum Offer {
    Switch { model: Box<dyn Model + Send>, context_window: usize, label: String },
    NoneLeft,
    Declined,
}

/// A helper's step, as the host shows it.
pub enum Step<'a> {
    Tool { name: &'a str, args: &'a Value },
    FellBack { to: &'a str, why: &'a str, failure: Failure },
}

/// The calling thread's side: makes helpers, shows their steps, answers
/// their fallback questions. `helper` is None when only one runs, else its
/// number from 1, in the order the model asked.
pub trait Host {
    /// A new helper's parts, for one task.
    fn make(&self) -> Helper;
    /// How many helpers may call the model at once.
    fn limit(&self) -> usize;
    fn show(&self, helper: Option<usize>, step: Step);
    /// Helper `index` (from 0) failed: the next model to try, if any.
    fn fall_back(&self, index: usize, why: &str, failure: Failure) -> Offer;
    /// A batch of `count` helpers starts (fallback state is per helper).
    fn begin(&self, _count: usize) {}
}

pub struct Delegate {
    host: Box<dyn Host>,
    /// What the helpers spent, for the caller to add to its own count.
    pub usage: Rc<Cell<Usage>>,
    /// The tool calls the helpers made, counted apart from the caller's own.
    pub calls: Rc<Cell<usize>>,
}

impl Delegate {
    pub fn new(host: Box<dyn Host>) -> Self {
        Delegate { host, usage: Rc::new(Cell::new(Usage::default())), calls: Rc::new(Cell::new(0)) }
    }

    /// Runs one helper per task, `Host::limit` at a time, and returns their
    /// results in the order of `tasks`.
    fn run_all(&self, tasks: Vec<String>) -> Vec<ToolResult> {
        let count = tasks.len();
        self.host.begin(count);
        let queue: Mutex<VecDeque<(usize, String, Helper)>> =
            Mutex::new(tasks.into_iter().enumerate().map(|(i, task)| (i, task, self.host.make())).collect());
        let workers = self.host.limit().clamp(1, count.max(1));
        let mut results: Vec<Option<Finished>> = (0..count).map(|_| None).collect();
        let (tx, rx) = channel::<Message>();
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (tx, queue) = (tx.clone(), &queue);
                scope.spawn(move || loop {
                    let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                    let Some((index, task, helper)) = next else { break };
                    let finished = run_helper(index, &task, helper, &tx);
                    let _ = tx.send(Message::Done(index, finished));
                });
            }
            drop(tx);
            let tag = |index: usize| (count > 1).then_some(index + 1);
            for message in rx {
                match message {
                    Message::Tool(index, name, args) => self.host.show(tag(index), Step::Tool { name: &name, args: &args }),
                    Message::FellBack(index, to, why, failure) => {
                        self.host.show(tag(index), Step::FellBack { to: &to, why: &why, failure })
                    }
                    Message::Ask(index, why, failure, reply) => {
                        let _ = reply.send(self.host.fall_back(index, &why, failure));
                    }
                    Message::Done(index, finished) => results[index] = Some(finished),
                }
            }
        });
        let mut spent = self.usage.get();
        let mut calls = self.calls.get();
        let out = results
            .into_iter()
            .map(|finished| {
                let finished = finished.unwrap_or_else(|| Finished::failed("The helper ended without an answer."));
                spent.add(finished.usage);
                calls += finished.tool_calls;
                finished.result
            })
            .collect();
        self.usage.set(spent);
        self.calls.set(calls);
        out
    }
}

/// What a helper thread sends back.
enum Message {
    Tool(usize, String, Value),
    FellBack(usize, String, String, Failure),
    Ask(usize, String, Failure, Sender<Offer>),
    Done(usize, Finished),
}

struct Finished {
    result: ToolResult,
    usage: Usage,
    tool_calls: usize,
}

impl Finished {
    fn failed(why: &str) -> Self {
        Finished { result: ToolResult::err(why), usage: Usage::default(), tool_calls: 0 }
    }
}

/// On a helper thread: builds the agent there (it is not thread-safe) and runs it.
fn run_helper(index: usize, task: &str, helper: Helper, tx: &Sender<Message>) -> Finished {
    let tools: Vec<Box<dyn Tool>> = helper.tools.into_iter().map(|t| t as Box<dyn Tool>).collect();
    let mut agent = Agent::new(helper.model, tools, helper.config);
    agent.stop = helper.stop;
    let events = tx.clone();
    agent.on_event(move |event| match event {
        Event::ToolStarted { name, args } => {
            let _ = events.send(Message::Tool(index, name.to_string(), (*args).clone()));
        }
        Event::FellBack { to, why, failure } => {
            let _ = events.send(Message::FellBack(index, to.to_string(), why.to_string(), *failure));
        }
        _ => {}
    });
    if helper.falls_back {
        let asks = tx.clone();
        agent.fallback = Some(Box::new(move |why, failure| {
            let (reply, answer) = channel();
            if asks.send(Message::Ask(index, why.to_string(), failure, reply)).is_err() {
                return Next::NoneLeft;
            }
            match answer.recv() {
                Ok(Offer::Switch { model, context_window, label }) => Next::Switch(Switch { model, context_window, label }),
                Ok(Offer::Declined) => Next::Declined,
                Ok(Offer::NoneLeft) | Err(_) => Next::NoneLeft,
            }
        }));
    }
    let result = agent.run(task);
    let footer = format!(
        "[helper: {} turn{}, {} tool call{}]",
        result.turns,
        if result.turns == 1 { "" } else { "s" },
        agent.tool_calls,
        if agent.tool_calls == 1 { "" } else { "s" }
    );
    let result = match result.outcome {
        Outcome::Done => ToolResult::ok(format!("{}\n{footer}", result.answer.trim())),
        Outcome::Stopped => ToolResult::err("The helper was stopped (Ctrl-C)."),
        Outcome::Failed(why) => ToolResult::err(format!("The helper failed: {why}")),
        other => {
            // Out of turns or looping: whatever it last said is still worth having.
            let said = agent.history().iter().rev().find_map(|m| {
                (m["role"] == "assistant").then(|| m["content"].as_str().unwrap_or("").trim().to_string()).filter(|t| !t.is_empty())
            });
            ToolResult::err(format!(
                "The helper did not finish ({other:?}). Its last words: {}\n{footer}",
                said.unwrap_or_else(|| "(nothing)".into())
            ))
        }
    };
    Finished { result, usage: agent.usage, tool_calls: agent.tool_calls }
}

fn task_of(args: &Value) -> Option<String> {
    str_arg(args, "task").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

impl Tool for Delegate {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "Give a self-contained question to a helper agent with a fresh context; you get back only its answer. Its only \
         tools are list_dir, search_files, code_map, read_file and fetch_url (and web_search when set up): no shell, no \
         edits. Several delegate calls in one reply run at the same time, so split independent questions (separate \
         folders, separate topics) into separate calls. Use it to search or understand a large codebase without filling \
         your own context. It sees none of this conversation: put everything it needs in `task`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "task": { "type": "string", "description": "The whole question, with the context it needs." } },
            "required": ["task"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        match task_of(args) {
            Some(task) => self.run_all(vec![task]).pop().unwrap_or_else(|| ToolResult::err("The helper did not run.")),
            None => ToolResult::err("delegate requires `task`."),
        }
    }
    fn runs_together(&self) -> bool {
        true
    }
    fn run_together(&self, calls: &[Value]) -> Vec<ToolResult> {
        // Calls without a task get their error; the rest run together.
        let tasks: Vec<Option<String>> = calls.iter().map(task_of).collect();
        let mut ran = self.run_all(tasks.iter().flatten().cloned().collect()).into_iter();
        tasks
            .iter()
            .map(|task| match task {
                Some(_) => ran.next().unwrap_or_else(|| ToolResult::err("The helper did not run.")),
                None => ToolResult::err("delegate requires `task`."),
            })
            .collect()
    }
}

/// Pure — unit-tested. How many helpers may call a model at once, from
/// Naseem's measured limits: a model on this machine serves one request at a
/// time; Ollama's cloud slowed to timeouts with three (2026-09-08); the
/// commercial APIs take three comfortably.
pub fn limit_for(base_url: &str, model: &str) -> usize {
    let after_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    let host = after_scheme.split(['/', ':']).next().unwrap_or("");
    let local = matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[")
        || host.starts_with("192.168.")
        || host.starts_with("10.")
        || host.ends_with(".local");
    let ollama_cloud = host.ends_with("ollama.com") || (local && (model.ends_with(":cloud") || model.ends_with("-cloud")));
    if ollama_cloud {
        2
    } else if local {
        1
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ModelError, Reply};
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    /// Answers once, after `wait`, with its text.
    struct Says(&'static str, Duration);
    impl Model for Says {
        fn chat(&mut self, _: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            std::thread::sleep(self.1);
            Ok(Reply { content: self.0.to_string(), usage: Usage { input: 7, output: 3, ..Default::default() }, ..Default::default() })
        }
    }

    /// Always out of credit.
    struct Fails;
    impl Model for Fails {
        fn chat(&mut self, _: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            Err(ModelError::Fatal("the model server answered 402: pay up".into()))
        }
    }

    struct TestHost {
        limit: usize,
        wait: Duration,
        failing: bool,
        asked: Rc<RefCell<Vec<usize>>>,
    }

    impl Host for TestHost {
        fn make(&self) -> Helper {
            let config = Config { system_prompt: format!("helper{HELPER_NOTE}"), max_turns: 5, context_window: 10_000 };
            let model: Box<dyn Model + Send> =
                if self.failing { Box::new(Fails) } else { Box::new(Says("Found it in src/a.rs:12.", self.wait)) };
            Helper { model, tools: Vec::new(), config, stop: Stop::new(), falls_back: self.failing }
        }
        fn limit(&self) -> usize {
            self.limit
        }
        fn show(&self, _: Option<usize>, _: Step) {}
        fn fall_back(&self, index: usize, _: &str, _: Failure) -> Offer {
            self.asked.borrow_mut().push(index);
            Offer::Switch {
                model: Box::new(Says("Answered on the second model.", Duration::ZERO)),
                context_window: 10_000,
                label: "b · m".into(),
            }
        }
    }

    fn host(limit: usize, wait: Duration) -> TestHost {
        TestHost { limit, wait, failing: false, asked: Rc::default() }
    }

    #[test]
    fn the_helper_answers_and_its_tokens_are_counted() {
        let tool = Delegate::new(Box::new(host(1, Duration::ZERO)));
        assert!(!tool.needs_approval());
        let answer = tool.run(&json!({ "task": "Where is the login check?" }));
        assert!(answer.ok && answer.text.starts_with("Found it in src/a.rs:12.\n[helper: 1 turn, 0 tool calls]"), "{}", answer.text);
        tool.run(&json!({ "task": "again" }));
        assert_eq!((tool.usage.get().input, tool.usage.get().output), (14, 6));
        assert_eq!(tool.calls.get(), 0, "Says makes no tool calls");
        assert!(!tool.run(&json!({})).ok);
    }

    #[test]
    fn helpers_in_one_reply_run_at_the_same_time_up_to_the_limit() {
        let wait = Duration::from_millis(300);
        let calls: Vec<Value> = (0..3).map(|i| json!({ "task": format!("question {i}") })).collect();
        // Three at once: about one wait in all.
        let together = Delegate::new(Box::new(host(3, wait)));
        let started = Instant::now();
        let results = together.run_together(&calls);
        let parallel = started.elapsed();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.ok));
        assert!(parallel < wait * 2, "three helpers took {parallel:?}");
        assert_eq!(together.usage.get().input, 21);
        // One at a time: about three waits.
        let serial = Delegate::new(Box::new(host(1, wait)));
        let started = Instant::now();
        serial.run_together(&calls);
        assert!(started.elapsed() >= wait * 3, "limit 1 must run them one by one");
    }

    #[test]
    fn a_call_without_a_task_gets_its_error_in_its_place() {
        let tool = Delegate::new(Box::new(host(2, Duration::ZERO)));
        let results = tool.run_together(&[json!({ "task": "a" }), json!({}), json!({ "task": "c" })]);
        assert_eq!(results.iter().map(|r| r.ok).collect::<Vec<_>>(), vec![true, false, true]);
        assert!(results[1].text.contains("requires `task`"));
    }

    #[test]
    fn a_failing_helper_asks_the_calling_thread_and_carries_on() {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let test = TestHost { limit: 2, wait: Duration::ZERO, failing: true, asked: asked.clone() };
        let tool = Delegate::new(Box::new(test));
        let results = tool.run_together(&[json!({ "task": "a" }), json!({ "task": "b" })]);
        assert!(results.iter().all(|r| r.ok && r.text.starts_with("Answered on the second model.")), "{}", results[0].text);
        let mut asked = asked.borrow().clone();
        asked.sort();
        assert_eq!(asked, vec![0, 1], "each helper asked once, from its own index");
    }

    #[test]
    fn the_limit_follows_the_provider() {
        assert_eq!(limit_for("http://localhost:11434/v1", "gemma4:31b"), 1);
        assert_eq!(limit_for("http://localhost:11434/v1", "kimi-k2.6:cloud"), 2);
        assert_eq!(limit_for("https://ollama.com/v1", "kimi-k2.6"), 2);
        assert_eq!(limit_for("https://api.anthropic.com/v1", "claude-haiku-5-5"), 3);
        assert_eq!(limit_for("http://127.0.0.1:1234/v1", "qwen"), 1);
    }
}
