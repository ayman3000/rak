//! delegate: hands a self-contained question to a helper agent with a fresh
//! context and read-only tools, and returns only its answer. Searching and
//! reading a large codebase then costs the main conversation one answer
//! instead of every file read on the way.
//!
//! From SwiftAgentKit's `delegate_task` and `SubAgentGate`: several
//! `delegate` calls in one reply run at the same time, each helper on its own
//! thread, at most `Host::limit` at once (the rest wait their turn). A helper
//! is read-only unless the call gives it `writes`, the paths it owns; writing
//! helpers in one call own separate paths, every edit or command they make is
//! approved by the person like the caller's own (`Host::approve`), and no
//! helper can delegate further.
//!
//! Everything that is not thread-safe stays on the calling thread: the
//! `Host` makes each helper's parts, prints its steps and answers its
//! fallback questions; the helper threads only send messages to it.

use crate::agent::{Agent, Approval, Config, Event, Outcome};
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
pub const WRITER_NOTE: &str = "\n\nYou are a working helper: another agent gave you one task and sees only your final reply. \
     You may change files only inside the paths you were assigned; everything else is read-only, and other helpers may be \
     working elsewhere at the same time. Do the work, run the relevant checks, then reply with: what you changed (file \
     paths), the commands you ran with their results, and anything left undone or needing a change outside your paths.";

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
    /// Input tokens it may spend before it must answer; None: no cap.
    pub input_budget: Option<u64>,
}

/// The model a helper carries on with after a failure.
pub enum Offer {
    Switch { model: Box<dyn Model + Send>, context_window: usize, label: String },
    NoneLeft,
    Declined,
}

/// A helper's step, as the host shows it.
pub enum Step<'a> {
    Tool {
        name: &'a str,
        args: &'a Value,
    },
    FellBack {
        to: &'a str,
        why: &'a str,
        failure: Failure,
    },
    /// The helper is done: one line on what it changed and ran.
    Summary(&'a str),
}

/// The calling thread's side: makes helpers, shows their steps, answers
/// their fallback questions. `helper` is None when only one runs, else its
/// number from 1, in the order the model asked.
pub trait Host {
    /// A new helper's parts, for one task. `writes`: the paths a writing
    /// helper may change; None for a read-only helper.
    fn make(&self, writes: Option<&[String]>) -> Helper;
    /// How many helpers may call the model at once.
    fn limit(&self) -> usize;
    fn show(&self, helper: Option<usize>, step: Step);
    /// Helper `index` (from 0) failed: the next model to try, if any.
    fn fall_back(&self, index: usize, why: &str, failure: Failure) -> Offer;
    /// A batch of `count` helpers starts (fallback state is per helper).
    fn begin(&self, _count: usize) {}
    /// A writing helper wants to run a tool that needs approval (an edit, a
    /// command): asked on the calling thread, where the person can answer.
    /// No unless the host says otherwise.
    fn approve(&self, _helper: Option<usize>, _name: &str, _args: &Value) -> Approval {
        Approval::No
    }
    /// A path given in `writes`, resolved as the host's file tools resolve it.
    fn resolve(&self, path: &str) -> std::path::PathBuf {
        crate::tools::normalize(std::path::Path::new(path))
    }
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
    fn run_all(&self, tasks: Vec<Task>) -> Vec<ToolResult> {
        let count = tasks.len();
        self.host.begin(count);
        if let Some(why) = overlap(&tasks, &|p| self.host.resolve(p)) {
            return tasks.iter().map(|_| ToolResult::err(why.clone())).collect();
        }
        let queue: Mutex<VecDeque<(usize, String, Helper)>> =
            Mutex::new(tasks.into_iter().enumerate().map(|(i, t)| (i, t.text, self.host.make(t.writes.as_deref()))).collect());
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
                    Message::Approve(index, name, args, reply) => {
                        let _ = reply.send(self.host.approve(tag(index), &name, &args));
                    }
                    Message::Done(index, finished) => {
                        self.host.show(tag(index), Step::Summary(&finished.summary));
                        results[index] = Some(finished);
                    }
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
    /// A tool that needs approval: the person's answer goes back on the sender.
    Approve(usize, String, Value, Sender<Approval>),
    Done(usize, Finished),
}

struct Finished {
    result: ToolResult,
    usage: Usage,
    tool_calls: usize,
    summary: String,
}

impl Finished {
    fn failed(why: &str) -> Self {
        Finished { result: ToolResult::err(why), usage: Usage::default(), tool_calls: 0, summary: "failed".into() }
    }
}

/// What a helper did, gathered from its finished tool calls.
#[derive(Default)]
struct Tally {
    files: Vec<String>,
    commands: Vec<(String, bool)>,
}

impl Tally {
    /// Records one finished call; only successful edits count as changes.
    fn add(&mut self, name: &str, args: &Value, ok: bool) {
        match name {
            "write_file" | "edit_file" | "apply_patch" if ok => {
                if let Some(path) = args["path"].as_str() {
                    if !self.files.iter().any(|f| f == path) {
                        self.files.push(path.to_string());
                    }
                }
            }
            "run_shell" => {
                if let Some(command) = args["command"].as_str() {
                    self.commands.push((command.to_string(), ok));
                }
            }
            _ => {}
        }
    }

    /// "done · 2 files changed (a.rs, b.rs) · ran 1 command: cargo test ✓"
    fn line(&self, ok: bool, tool_calls: usize) -> String {
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        // Counts first, so a narrow terminal that cuts the line still shows them.
        let failed = self.commands.iter().filter(|(_, ok)| !ok).count();
        let mut head = format!(
            "{} · {} changed · ran {}",
            if ok { "done" } else { "failed" },
            plural(self.files.len(), "file"),
            plural(self.commands.len(), "command")
        );
        if failed > 0 {
            head.push_str(&format!(" ({failed} failed)"));
        }
        let mut parts = vec![head, plural(tool_calls, "tool call")];
        if !self.files.is_empty() {
            parts.push(shorten(&self.files.join(", "), 60));
        }
        if !self.commands.is_empty() {
            let shown: Vec<String> = self
                .commands
                .iter()
                .map(|(c, ok)| format!("{} {}", shorten(c.lines().next().unwrap_or(""), 30), if *ok { "✓" } else { "✗" }))
                .collect();
            parts.push(shorten(&shown.join("; "), 70));
        }
        parts.join(" · ")
    }
}

/// Cuts `text` to `max` characters, marking the cut.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max.saturating_sub(1)).collect::<String>())
}

/// On a helper thread: builds the agent there (it is not thread-safe) and runs it.
fn run_helper(index: usize, task: &str, helper: Helper, tx: &Sender<Message>) -> Finished {
    let tools: Vec<Box<dyn Tool>> = helper.tools.into_iter().map(|t| t as Box<dyn Tool>).collect();
    let mut agent = Agent::new(helper.model, tools, helper.config);
    agent.stop = helper.stop;
    agent.input_budget = helper.input_budget;
    // Approvals are the person's, on the calling thread: an edit or command
    // waits for the answer, and no answer (the host gone) is a no.
    let asks = tx.clone();
    agent.on_approval(move |name, args| {
        let (reply, answer) = channel();
        if asks.send(Message::Approve(index, name.to_string(), args.clone(), reply)).is_err() {
            return Approval::No;
        }
        answer.recv().unwrap_or(Approval::No)
    });
    let events = tx.clone();
    let tally = std::sync::Arc::new(Mutex::new(Tally::default()));
    let (tallied, pending) = (tally.clone(), std::sync::Arc::new(Mutex::new(Vec::<Value>::new())));
    agent.on_event(move |event| match event {
        Event::ToolStarted { name, args } => {
            pending.lock().unwrap_or_else(|e| e.into_inner()).push((*args).clone());
            let _ = events.send(Message::Tool(index, name.to_string(), (*args).clone()));
        }
        Event::ToolFinished { name, ok, text, .. } => {
            let mut started = pending.lock().unwrap_or_else(|e| e.into_inner());
            let args = if started.is_empty() { Value::Null } else { started.remove(0) };
            // run_shell reports a failing command as a finished call; its exit line tells.
            let ok = *ok && (*name != "run_shell" || text.starts_with("exit 0"));
            tallied.lock().unwrap_or_else(|e| e.into_inner()).add(name, &args, ok);
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
        // Some models stop with nothing after a large read; say so instead of
        // passing on an answer that is only the footer.
        Outcome::Done if result.answer.trim().is_empty() => ToolResult::err(format!(
            "The helper ended with an empty reply, so there is no answer. Ask again with a narrower question, or read \
             the files yourself.\n{footer}"
        )),
        Outcome::Done => ToolResult::ok(format!("{}\n{footer}", result.answer.trim())),
        // What it found by then is still an answer, marked as possibly short.
        Outcome::OverBudget => ToolResult::ok(format!(
            "{}\n[The helper reached its budget of {} input tokens and answered with what it had; this may be \
             incomplete. Ask a narrower question for what is missing.]\n{footer}",
            result.answer.trim(),
            helper.input_budget.unwrap_or(0)
        )),
        Outcome::Stopped => ToolResult::err("The helper was stopped (Ctrl-C)."),
        Outcome::Failed(why) => ToolResult::err(format!("The helper failed: {why}")),
        // Like the caller's own refusal: stop, and don't look for another way.
        Outcome::Refused { tool } => ToolResult::err(format!(
            "The person did not approve the helper's {tool}, so the helper stopped there; nothing was changed by that \
             call. Don't try to make the change another way: tell the person what was not done.\n{footer}"
        )),
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
    let summary = tally.lock().unwrap_or_else(|e| e.into_inner()).line(result.ok, agent.tool_calls);
    Finished { result, usage: agent.usage, tool_calls: agent.tool_calls, summary }
}

/// One delegated job: the task, and for a writing helper the paths it owns.
#[derive(Clone)]
pub struct Task {
    pub text: String,
    pub writes: Option<Vec<String>>,
}

fn task_of(args: &Value) -> Option<Task> {
    let text = str_arg(args, "task").map(str::trim).filter(|t| !t.is_empty())?.to_string();
    let writes: Option<Vec<String>> = args.get("writes").and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(Value::as_str)
            .map(|p| p.trim().trim_end_matches('/').to_string())
            .filter(|p| !p.is_empty() && p != ".")
            .collect()
    });
    Some(Task { text, writes: writes.filter(|w| !w.is_empty()) })
}

/// Pure — unit-tested. Writing helpers that run together must own separate
/// paths, or they overwrite each other: why not, when two lanes overlap.
/// Paths are compared as `resolve` gives them (the host resolves them as its
/// file tools do) and ignoring case, as macOS disks usually do: a pair that
/// may be one folder counts as one.
pub fn overlap(tasks: &[Task], resolve: &dyn Fn(&str) -> std::path::PathBuf) -> Option<String> {
    let lanes: Vec<&String> = tasks.iter().filter_map(|t| t.writes.as_ref()).flatten().collect();
    let key = |p: &str| std::path::PathBuf::from(resolve(p).to_string_lossy().to_lowercase());
    let inside = |a: &str, b: &str| key(a).starts_with(key(b));
    for (i, a) in lanes.iter().enumerate() {
        for b in &lanes[i + 1..] {
            if inside(a, b) || inside(b, a) {
                return Some(format!(
                    "Writing helpers in one reply must own separate paths, but `{a}` and `{b}` overlap. Give each helper its \
                     own folder or files, or run the overlapping work in one helper."
                ));
            }
        }
    }
    None
}

impl Tool for Delegate {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "Give a self-contained task to a helper agent with a fresh context; you get back only its answer. Without \
         `writes` it is a read-only researcher (list_dir, search_files, code_map, read_file, fetch_url): use it to examine \
         folders or a large codebase without filling your own context. With `writes` (paths it owns, e.g. [\"frontend\"]) \
         it is a worker that may also edit files and run commands, but only change files inside those paths — use it for \
         separate work such as one helper for the frontend and one for the backend. Several delegate calls in one reply \
         run at the same time; writing helpers in one reply must own separate paths. It sees none of this conversation: \
         put everything it needs in `task`, and review what it changed before you rely on it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "The whole task, with the context it needs." },
                "writes": { "type": "array", "items": { "type": "string" }, "description": "Optional: files or folders this helper owns and may change. Omit for a read-only helper." }
            },
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
        let tasks: Vec<Option<Task>> = calls.iter().map(task_of).collect();
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
        budget: Option<u64>,
    }

    impl Host for TestHost {
        fn make(&self, _: Option<&[String]>) -> Helper {
            let config = Config { system_prompt: format!("helper{HELPER_NOTE}"), max_turns: 5, context_window: 10_000 };
            let model: Box<dyn Model + Send> =
                if self.failing { Box::new(Fails) } else { Box::new(Says("Found it in src/a.rs:12.", self.wait)) };
            Helper { model, tools: Vec::new(), config, stop: Stop::new(), falls_back: self.failing, input_budget: self.budget }
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
        TestHost { limit, wait, failing: false, asked: Rc::default(), budget: None }
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

    struct Silent;
    impl Model for Silent {
        fn chat(&mut self, _: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            Ok(Reply::default())
        }
    }

    struct SilentHost;
    impl Host for SilentHost {
        fn make(&self, _: Option<&[String]>) -> Helper {
            let config = Config { system_prompt: "helper".into(), max_turns: 5, context_window: 10_000 };
            Helper { model: Box::new(Silent), tools: Vec::new(), config, stop: Stop::new(), falls_back: false, input_budget: None }
        }
        fn limit(&self) -> usize {
            1
        }
        fn show(&self, _: Option<usize>, _: Step) {}
        fn fall_back(&self, _: usize, _: &str, _: Failure) -> Offer {
            Offer::NoneLeft
        }
    }

    #[test]
    fn an_empty_answer_comes_back_as_an_error_not_as_a_bare_footer() {
        let result = Delegate::new(Box::new(SilentHost)).run(&json!({ "task": "x" }));
        assert!(!result.ok && result.text.starts_with("The helper ended with an empty reply"), "{}", result.text);
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
        let test = TestHost { limit: 2, wait: Duration::ZERO, failing: true, asked: asked.clone(), budget: None };
        let tool = Delegate::new(Box::new(test));
        let results = tool.run_together(&[json!({ "task": "a" }), json!({ "task": "b" })]);
        assert!(results.iter().all(|r| r.ok && r.text.starts_with("Answered on the second model.")), "{}", results[0].text);
        let mut asked = asked.borrow().clone();
        asked.sort();
        assert_eq!(asked, vec![0, 1], "each helper asked once, from its own index");
    }

    /// Keeps asking to list a folder; each call costs 100 input tokens.
    struct Wanders;
    impl Model for Wanders {
        fn chat(&mut self, messages: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            let usage = Usage { input: 100, ..Default::default() };
            let told_to_answer = messages.last().and_then(|m| m["content"].as_str()).is_some_and(|t| t.contains("Do not call any tools"));
            if told_to_answer {
                return Ok(Reply { content: "Partial answer.".into(), usage, ..Default::default() });
            }
            let call = crate::provider::ToolCall { id: "c".into(), name: "none".into(), arguments: "{}".into() };
            Ok(Reply { tool_calls: vec![call], usage, ..Default::default() })
        }
    }

    struct WanderingHost;
    impl Host for WanderingHost {
        fn make(&self, _: Option<&[String]>) -> Helper {
            let config = Config { system_prompt: "helper".into(), max_turns: 25, context_window: 10_000 };
            Helper { model: Box::new(Wanders), tools: Vec::new(), config, stop: Stop::new(), falls_back: false, input_budget: Some(250) }
        }
        fn limit(&self) -> usize {
            1
        }
        fn show(&self, _: Option<usize>, _: Step) {}
        fn fall_back(&self, _: usize, _: &str, _: Failure) -> Offer {
            Offer::NoneLeft
        }
    }

    /// Calls `write_file` once, then answers.
    struct Writes;
    impl Model for Writes {
        fn chat(&mut self, messages: &[Value], _: &[Value]) -> Result<Reply, ModelError> {
            if messages.iter().any(|m| m["role"] == "tool") {
                return Ok(Reply { content: "Finished.".into(), ..Default::default() });
            }
            let call = crate::provider::ToolCall { id: "w".into(), name: "write_file".into(), arguments: "{\"path\":\"a.txt\"}".into() };
            Ok(Reply { tool_calls: vec![call], ..Default::default() })
        }
    }

    /// A stand-in for an editing tool: records that it ran.
    struct Edits(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Tool for Edits {
        fn name(&self) -> &str {
            "write_file"
        }
        fn description(&self) -> &str {
            "writes"
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object" })
        }
        fn needs_approval(&self) -> bool {
            true
        }
        fn run(&self, _: &Value) -> ToolResult {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            ToolResult::ok("written")
        }
    }

    struct WritingHost {
        answer: Approval,
        ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
        asked: Rc<RefCell<Vec<String>>>,
    }
    impl Host for WritingHost {
        fn make(&self, _: Option<&[String]>) -> Helper {
            let config = Config { system_prompt: format!("helper{WRITER_NOTE}"), max_turns: 5, context_window: 10_000 };
            let tools: Vec<Box<dyn Tool + Send>> = vec![Box::new(Edits(self.ran.clone()))];
            Helper { model: Box::new(Writes), tools, config, stop: Stop::new(), falls_back: false, input_budget: None }
        }
        fn limit(&self) -> usize {
            1
        }
        fn show(&self, _: Option<usize>, _: Step) {}
        fn fall_back(&self, _: usize, _: &str, _: Failure) -> Offer {
            Offer::NoneLeft
        }
        fn approve(&self, _: Option<usize>, name: &str, _: &Value) -> Approval {
            self.asked.borrow_mut().push(name.to_string());
            self.answer
        }
    }

    fn run_writer(answer: Approval) -> (ToolResult, bool, Vec<String>) {
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let asked = Rc::new(RefCell::new(Vec::new()));
        let tool = Delegate::new(Box::new(WritingHost { answer, ran: ran.clone(), asked: asked.clone() }));
        let r = tool.run(&json!({ "task": "write a.txt", "writes": ["."] }));
        let asked = asked.borrow().clone();
        (r, ran.load(std::sync::atomic::Ordering::SeqCst), asked)
    }

    /// A writing helper's edits and commands are asked about like the main
    /// agent's: delegating is not a way around approval.
    #[test]
    fn a_writing_helper_asks_before_it_edits_and_a_no_stops_the_edit() {
        let (r, ran, asked) = run_writer(Approval::No);
        assert_eq!(asked, ["write_file"], "the person was not asked");
        assert!(!ran, "the edit ran although it was refused");
        assert!(!r.ok && r.text.contains("did not approve the helper's write_file"), "{}", r.text);
        let (r, ran, asked) = run_writer(Approval::Yes);
        assert!(r.ok && ran && asked == ["write_file"], "an approved edit did not run: {}", r.text);
    }

    #[test]
    fn a_helper_over_its_budget_answers_with_what_it_has() {
        let tool = Delegate::new(Box::new(WanderingHost));
        let result = tool.run(&json!({ "task": "look everywhere" }));
        assert!(result.ok, "{}", result.text);
        assert!(result.text.starts_with("Partial answer.\n[The helper reached its budget of 250 input tokens"), "{}", result.text);
        assert_eq!(tool.usage.get().input, 400, "three calls to pass 250, then the answer");
    }

    #[test]
    fn the_limit_follows_the_provider() {
        assert_eq!(limit_for("http://localhost:11434/v1", "gemma4:31b"), 1);
        assert_eq!(limit_for("http://localhost:11434/v1", "kimi-k2.6:cloud"), 2);
        assert_eq!(limit_for("https://ollama.com/v1", "kimi-k2.6"), 2);
        assert_eq!(limit_for("https://api.anthropic.com/v1", "claude-haiku-5-5"), 3);
        assert_eq!(limit_for("http://127.0.0.1:1234/v1", "qwen"), 1);
    }

    fn task(writes: Option<&[&str]>) -> Task {
        Task { text: "t".into(), writes: writes.map(|w| w.iter().map(|s| s.to_string()).collect()) }
    }

    #[test]
    fn writing_helpers_must_own_separate_paths() {
        let overlap = |tasks: &[Task]| overlap(tasks, &in_project);
        assert!(overlap(&[task(Some(&["frontend"])), task(Some(&["backend"])), task(None), task(None)]).is_none());
        assert!(overlap(&[task(Some(&["src"])), task(Some(&["src/api"]))]).is_some());
        assert!(overlap(&[task(Some(&["web"])), task(Some(&["web"]))]).is_some());
        assert!(overlap(&[task(Some(&["web"])), task(Some(&["webapp"]))]).is_none(), "a shared prefix is not a shared folder");
    }

    /// Paths as the host resolves them, in a project at /proj.
    fn in_project(path: &str) -> std::path::PathBuf {
        crate::tools::normalize(&std::path::Path::new("/proj").join(path))
    }

    /// One folder written different ways is still one folder.
    #[test]
    fn the_same_folder_spelled_differently_still_overlaps() {
        let overlap = |a: &str, b: &str| overlap(&[task(Some(&[a])), task(Some(&[b]))], &in_project).is_some();
        assert!(overlap("./frontend", "frontend"));
        assert!(overlap("frontend/../frontend", "frontend"));
        assert!(overlap("/proj/frontend", "frontend/app"));
        assert!(overlap("Web", "web"), "macOS disks usually ignore case");
        assert!(!overlap("./frontend", "./backend"));
    }

    #[test]
    fn writes_are_read_from_the_call() {
        let t = task_of(&json!({ "task": "build it", "writes": ["frontend/", " ", "."] })).unwrap();
        assert_eq!(t.writes, Some(vec!["frontend".to_string()]));
        assert_eq!(task_of(&json!({ "task": "look" })).unwrap().writes, None);
        assert_eq!(task_of(&json!({ "task": "look", "writes": [] })).unwrap().writes, None);
    }

    #[test]
    fn the_summary_lists_changed_files_and_commands() {
        let mut t = Tally::default();
        t.add("edit_file", &json!({"path": "backend/api.py"}), true);
        t.add("edit_file", &json!({"path": "backend/api.py"}), true);
        t.add("write_file", &json!({"path": "nope.txt"}), false);
        t.add("read_file", &json!({"path": "frontend/ui.js"}), true);
        t.add("run_shell", &json!({"command": "python3 -m pytest\nmore"}), true);
        t.add("run_shell", &json!({"command": "echo x >> README.txt"}), false);
        assert_eq!(
            t.line(true, 6),
            "done · 1 file changed · ran 2 commands (1 failed) · 6 tool calls · backend/api.py · python3 -m pytest ✓; echo x >> README.txt ✗"
        );
        assert_eq!(Tally::default().line(false, 1), "failed · 0 files changed · ran 0 commands · 1 tool call");
    }
}
