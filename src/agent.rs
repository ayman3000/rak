//! The agent loop: ask the model, run the tools it calls, give it the results,
//! repeat until it answers without calling a tool.

use crate::interrupt::Stop;
use crate::provider::{Model, ModelError, Reply, Usage};
use crate::tools::{definition, Tool};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Config {
    pub system_prompt: String,
    /// Model calls allowed for one request.
    pub max_turns: usize,
    /// The model's context window in tokens. Old tool output is dropped as the
    /// conversation nears it.
    pub context_window: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Approval {
    Yes,
    No,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done,
    /// The turn limit was reached; the answer is the model's own summary.
    MaxTurns,
    /// The person said no to this tool (or nobody was there to ask).
    Refused { tool: String },
    /// The model kept making the same call with the same result.
    Loop,
    /// The person stopped the request (Ctrl-C).
    Stopped,
    Failed(String),
}

#[derive(Debug)]
pub struct RunResult {
    pub outcome: Outcome,
    pub answer: String,
    pub turns: usize,
}

pub enum Event<'a> {
    ModelCalled { turn: usize, usage: Usage, millis: u128 },
    Retrying { attempt: usize, error: &'a str },
    /// A piece of the model's text as it arrives (only when `stream` is on).
    Delta(&'a str),
    /// Text the model wrote alongside tool calls, whole.
    Text(&'a str),
    ToolStarted { name: &'a str, args: &'a Value },
    ToolFinished { name: &'a str, ok: bool, denied: bool, text: &'a str, millis: u128 },
    Note(&'a str),
}

const STOP_NOTE: &str = "Not run: the user stopped the request.";

pub const REFUSAL_NOTE: &str = "Not run: the user said no to this. Do not retry it and do not do the same \
    thing another way. Wait for the user's next message.";

const MAX_RETRIES: usize = 5;
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(8);
/// The same call with the same result this many times in a row draws a warning…
const LOOP_WARN: usize = 3;
/// …and this many ends the request.
const LOOP_STOP: usize = 6;
/// Tool results in the most recent messages are never dropped.
const KEEP_RECENT: usize = 8;
const DROPPED: &str = "[earlier output removed to save context; run the tool again if you need it]";

pub struct Agent {
    model: Box<dyn Model>,
    tools: Vec<Box<dyn Tool>>,
    config: Config,
    /// The conversation, in chat-completions shape. Kept between requests.
    pub messages: Vec<Value>,
    pub usage: Usage,
    pub tool_calls: usize,
    pub retry_base: Duration,
    /// Ask the model for its text piece by piece and pass each on as `Event::Delta`.
    pub stream: bool,
    /// Set from outside (Ctrl-C) to end the request at the next step.
    pub stop: Stop,
    /// Input tokens of the latest model call: how full the context is.
    pub last_prompt_tokens: u64,
    approve: Box<dyn FnMut(&str, &Value) -> Approval>,
    on_event: Box<dyn for<'a> FnMut(&Event<'a>)>,
}

impl Agent {
    pub fn new(model: Box<dyn Model>, tools: Vec<Box<dyn Tool>>, config: Config) -> Self {
        let messages = vec![json!({ "role": "system", "content": config.system_prompt })];
        Agent {
            model,
            tools,
            config,
            messages,
            usage: Usage::default(),
            tool_calls: 0,
            retry_base: Duration::from_secs(1),
            stream: false,
            stop: Stop::new(),
            last_prompt_tokens: 0,
            approve: Box::new(|_, _| Approval::Yes),
            on_event: Box::new(|_| {}),
        }
    }

    /// Asked before every tool that changes files or runs commands.
    pub fn on_approval(&mut self, approve: impl FnMut(&str, &Value) -> Approval + 'static) {
        self.approve = Box::new(approve);
    }

    pub fn on_event(&mut self, on_event: impl for<'a> FnMut(&Event<'a>) + 'static) {
        self.on_event = Box::new(on_event);
    }

    pub fn tool_names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    pub fn tools(&self) -> &[Box<dyn Tool>] {
        &self.tools
    }

    /// Another model from here on; the conversation so far comes along.
    pub fn set_model(&mut self, model: Box<dyn Model>) {
        self.model = model;
    }

    /// The model's context window in tokens, once it is known.
    pub fn set_context_window(&mut self, tokens: usize) {
        self.config.context_window = tokens;
    }

    /// Forget the conversation; the standing instructions stay.
    pub fn clear(&mut self) {
        self.messages.truncate(1);
        self.last_prompt_tokens = 0;
    }

    fn stopped(&mut self, turn: usize) -> RunResult {
        // Keep the conversation well formed for the next request.
        if self.messages.last().is_some_and(|m| m["role"] == "user") {
            self.messages.push(json!({ "role": "assistant", "content": "(stopped by the user)" }));
        }
        RunResult { outcome: Outcome::Stopped, answer: String::new(), turns: turn }
    }

    pub fn run(&mut self, task: &str) -> RunResult {
        self.messages.push(json!({ "role": "user", "content": task }));
        let definitions: Vec<Value> = self.tools.iter().map(|t| definition(t.as_ref())).collect();
        let nudge_at = (self.config.max_turns >= 10).then(|| self.config.max_turns * 4 / 5);
        let mut repeated: (String, usize) = (String::new(), 0);
        let mut empty_replies = 0;

        for turn in 1..=self.config.max_turns {
            let dropped = trim_context(&mut self.messages, self.config.context_window * 3);
            if dropped > 0 {
                (self.on_event)(&Event::Note(&format!("context is filling up: dropped {dropped} old tool outputs")));
            }
            if self.stop.requested() {
                return self.stopped(turn);
            }
            let reply = match self.call_model(turn, &definitions) {
                Ok(r) => r,
                Err(_) if self.stop.requested() => return self.stopped(turn),
                Err(e) => return RunResult { outcome: Outcome::Failed(e), answer: String::new(), turns: turn },
            };

            if reply.tool_calls.is_empty() {
                if reply.content.trim().is_empty() && empty_replies == 0 {
                    // Some models stop with nothing after a tool result. Ask once.
                    empty_replies += 1;
                    self.messages.push(json!({ "role": "assistant", "content": "" }));
                    self.messages.push(json!({ "role": "user", "content":
                        "You sent an empty reply. Continue the task, or give your final summary if it is done." }));
                    continue;
                }
                self.messages.push(json!({ "role": "assistant", "content": reply.content }));
                return RunResult { outcome: Outcome::Done, answer: reply.content, turns: turn };
            }

            self.messages.push(assistant_message(&reply));
            if !reply.content.trim().is_empty() {
                (self.on_event)(&Event::Text(&reply.content));
            }

            let mut refused: Option<String> = None;
            for call in &reply.tool_calls {
                let mut result = if refused.is_some() {
                    (false, REFUSAL_NOTE.to_string())
                } else if self.stop.requested() {
                    (false, STOP_NOTE.to_string())
                } else {
                    self.run_tool(&call.name, &call.arguments, &mut refused)
                };

                // The same call with the same result, again and again.
                let signature = format!("{}\u{1}{}\u{1}{}", call.name, call.arguments, result.1);
                repeated = if repeated.0 == signature { (signature, repeated.1 + 1) } else { (signature, 1) };
                if repeated.1 >= LOOP_WARN {
                    result.1.push_str(&format!(
                        "\n[You have made this exact call {} times and got the same result. Repeating it will \
                         not change anything — try a different approach, or finish with what you have.]",
                        repeated.1
                    ));
                }
                self.messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": result.1 }));
            }
            if let Some(tool) = refused {
                return RunResult { outcome: Outcome::Refused { tool }, answer: String::new(), turns: turn };
            }
            if self.stop.requested() {
                return self.stopped(turn);
            }
            if repeated.1 >= LOOP_STOP {
                return RunResult { outcome: Outcome::Loop, answer: String::new(), turns: turn };
            }
            if nudge_at == Some(turn) {
                self.messages.push(json!({ "role": "user", "content": format!(
                    "[progress note] You have used {turn} of {} turns. Finish the essential work now and \
                     leave time to give your summary.", self.config.max_turns) }));
            }
        }

        // Out of turns: one last call without tools, for a summary of where things stand.
        self.messages.push(json!({ "role": "user", "content":
            "You have reached the turn limit. Do not call any tools. Say briefly what you did and what is left to do." }));
        let turns = self.config.max_turns;
        let answer = match self.call_model(turns + 1, &[]) {
            Ok(reply) => {
                self.messages.push(json!({ "role": "assistant", "content": reply.content }));
                reply.content
            }
            Err(_) => String::new(),
        };
        RunResult { outcome: Outcome::MaxTurns, answer, turns }
    }

    fn call_model(&mut self, turn: usize, definitions: &[Value]) -> Result<Reply, String> {
        let mut attempt = 0;
        loop {
            let started = Instant::now();
            let result = if self.stream {
                let on_event = &mut self.on_event;
                self.model.chat_stream(&self.messages, definitions, &mut |piece| on_event(&Event::Delta(piece)))
            } else {
                self.model.chat(&self.messages, definitions)
            };
            match result {
                Ok(reply) => {
                    self.last_prompt_tokens = reply.usage.input;
                    self.usage.input += reply.usage.input;
                    self.usage.output += reply.usage.output;
                    (self.on_event)(&Event::ModelCalled { turn, usage: reply.usage, millis: started.elapsed().as_millis() });
                    return Ok(reply);
                }
                Err(ModelError::Retryable(message)) if attempt < MAX_RETRIES && !self.stop.requested() => {
                    attempt += 1;
                    (self.on_event)(&Event::Retrying { attempt, error: &message });
                    std::thread::sleep((self.retry_base * 2u32.pow(attempt as u32 - 1)).min(MAX_RETRY_BACKOFF));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// Runs one tool call and returns (ok, text for the model). Sets `refused`
    /// when the person said no.
    fn run_tool(&mut self, name: &str, arguments: &str, refused: &mut Option<String>) -> (bool, String) {
        let Some(tool) = self.tools.iter().find(|t| t.name() == name) else {
            return (false, format!("There is no tool named `{name}`. The tools are: {}.", self.tool_names().join(", ")));
        };
        let args: Value = match serde_json::from_str(if arguments.trim().is_empty() { "{}" } else { arguments }) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) => return (false, format!("The arguments for {name} must be a JSON object.")),
            Err(e) => return (false, format!("The arguments for {name} were not valid JSON ({e}). Send one JSON object.")),
        };
        self.tool_calls += 1;
        (self.on_event)(&Event::ToolStarted { name, args: &args });

        if tool.needs_approval() && (self.approve)(name, &args) == Approval::No {
            *refused = Some(name.to_string());
            (self.on_event)(&Event::ToolFinished { name, ok: false, denied: true, text: REFUSAL_NOTE, millis: 0 });
            return (false, REFUSAL_NOTE.to_string());
        }
        let started = Instant::now();
        let result = tool.run(&args);
        (self.on_event)(&Event::ToolFinished {
            name,
            ok: result.ok,
            denied: false,
            text: &result.text,
            millis: started.elapsed().as_millis(),
        });
        (result.ok, result.text)
    }
}

fn assistant_message(reply: &Reply) -> Value {
    let calls: Vec<Value> = reply
        .tool_calls
        .iter()
        .map(|c| json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments } }))
        .collect();
    let content = if reply.content.is_empty() { Value::Null } else { json!(reply.content) };
    json!({ "role": "assistant", "content": content, "tool_calls": calls })
}

/// Pure — unit-tested. When the conversation is longer than `budget_chars`,
/// the oldest tool outputs are replaced by a one-line stub, oldest first, until
/// it fits. The most recent messages are never touched. Returns how many
/// outputs were dropped.
pub fn trim_context(messages: &mut [Value], budget_chars: usize) -> usize {
    let size = |m: &Value| m.to_string().len();
    let mut total: usize = messages.iter().map(size).sum();
    let last_droppable = messages.len().saturating_sub(KEEP_RECENT);
    let mut dropped = 0;
    for message in messages[..last_droppable].iter_mut() {
        if total <= budget_chars {
            break;
        }
        let is_big_tool_output = message["role"] == "tool"
            && message["content"].as_str().is_some_and(|c| c.len() > DROPPED.len() * 2);
        if is_big_tool_output {
            let before = size(message);
            message["content"] = json!(DROPPED);
            total -= before - size(message);
            dropped += 1;
        }
    }
    dropped
}

/// The standing instructions. Follows sak's prompt.
pub fn system_prompt(cwd: &Path, read_only: bool, project_notes: Option<&str>) -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    };
    let mut prompt = format!(
        "You are rak, a coding agent working in a terminal on {os}.\n\
         The project folder is {}. Relative paths resolve there.\n\
         Look before you change: list and read files first. Make focused edits.\n\
         After changing code, build or test it with run_shell when that makes sense.\n\
         Answer concisely; Markdown is fine (headings, bullets, code).\n\
         Make the code pass the tests, never the tests pass the code: don't edit or delete a test you were \
         asked to make pass; adding a test is fine. If a test looks wrong, say so and stop.\n\
         Finish with a short summary of what you did.",
        cwd.display()
    );
    if read_only {
        prompt.push_str(
            "\n\nYou are in READ-ONLY mode: you can read files and search, but not edit files or run \
             commands. Report what you find and what you would change.",
        );
    }
    if let Some(notes) = project_notes.map(str::trim).filter(|n| !n.is_empty()) {
        prompt.push_str("\n\nThe project's own notes for agents (AGENTS.md):\n\n");
        prompt.push_str(notes);
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolCall;
    use crate::tools::{core_tools, testing::temp_dir, Ctx};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A model that plays back a fixed list of replies and records what it was sent.
    struct Script {
        replies: Vec<Result<Reply, ModelError>>,
        seen: Rc<RefCell<Vec<Vec<Value>>>>,
    }

    impl Model for Script {
        fn chat(&mut self, messages: &[Value], _tools: &[Value]) -> Result<Reply, ModelError> {
            self.seen.borrow_mut().push(messages.to_vec());
            if self.replies.is_empty() {
                return Ok(text("(script ran out)"));
            }
            self.replies.remove(0)
        }
    }

    fn text(content: &str) -> Reply {
        Reply { content: content.into(), tool_calls: vec![], usage: Usage { input: 10, output: 2 } }
    }

    fn call(name: &str, args: Value) -> Reply {
        calls(vec![(name, args)])
    }

    fn calls(list: Vec<(&str, Value)>) -> Reply {
        let tool_calls = list
            .into_iter()
            .enumerate()
            .map(|(i, (name, args))| ToolCall { id: format!("c{i}"), name: name.into(), arguments: args.to_string() })
            .collect();
        Reply { content: String::new(), tool_calls, usage: Usage { input: 10, output: 2 } }
    }

    struct Setup {
        agent: Agent,
        seen: Rc<RefCell<Vec<Vec<Value>>>>,
        dir: std::path::PathBuf,
    }

    fn setup(label: &str, replies: Vec<Result<Reply, ModelError>>, max_turns: usize) -> Setup {
        let dir = temp_dir(label);
        let ctx = Ctx::new(&dir, false);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let model = Script { replies, seen: seen.clone() };
        let config = Config { system_prompt: "sys".into(), max_turns, context_window: 100_000 };
        let mut agent = Agent::new(Box::new(model), core_tools(&ctx, false), config);
        agent.retry_base = Duration::ZERO;
        Setup { agent, seen, dir }
    }

    #[test]
    fn tools_run_and_their_results_go_back_to_the_model() {
        let mut s = setup(
            "loop",
            vec![
                Ok(call("write_file", json!({ "path": "hi.txt", "content": "hello" }))),
                Ok(call("read_file", json!({ "path": "hi.txt" }))),
                Ok(text("Done: wrote hi.txt.")),
            ],
            10,
        );
        let result = s.agent.run("make hi.txt");
        assert_eq!(result.outcome, Outcome::Done);
        assert_eq!(result.answer, "Done: wrote hi.txt.");
        assert_eq!(result.turns, 3);
        assert_eq!(std::fs::read_to_string(s.dir.join("hi.txt")).unwrap(), "hello");
        assert_eq!(s.agent.usage, Usage { input: 30, output: 6 });
        assert_eq!(s.agent.tool_calls, 2);

        // The third request carries both tool results, matched to their calls.
        let third = &s.seen.borrow()[2];
        let roles: Vec<&str> = third.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "tool"]);
        assert_eq!(third[3]["tool_call_id"], "c0");
        assert_eq!(third[5]["content"], "hello");
    }

    #[test]
    fn a_no_stops_the_request_and_every_call_still_gets_a_result() {
        let mut s = setup(
            "refuse",
            vec![Ok(calls(vec![
                ("write_file", json!({ "path": "a.txt", "content": "x" })),
                ("run_shell", json!({ "command": "echo x > a.txt" })),
            ]))],
            10,
        );
        s.agent.on_approval(|_, _| Approval::No);
        let result = s.agent.run("write a.txt");
        assert_eq!(result.outcome, Outcome::Refused { tool: "write_file".into() });
        assert!(!s.dir.join("a.txt").exists());
        assert_eq!(s.seen.borrow().len(), 1, "the model must not get another turn after a no");
        let tool_results: Vec<&Value> = s.agent.messages.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(tool_results.len(), 2);
        assert!(tool_results.iter().all(|m| m["content"] == REFUSAL_NOTE));
    }

    #[test]
    fn reading_never_asks_for_approval() {
        let mut s = setup("readonly-approval", vec![Ok(call("list_dir", json!({}))), Ok(text("empty"))], 10);
        s.agent.on_approval(|_, _| panic!("list_dir must not ask"));
        assert_eq!(s.agent.run("look").outcome, Outcome::Done);
    }

    #[test]
    fn bad_arguments_and_unknown_tools_are_explained_to_the_model() {
        let bad = Reply {
            content: String::new(),
            tool_calls: vec![
                ToolCall { id: "a".into(), name: "read_file".into(), arguments: "{not json".into() },
                ToolCall { id: "b".into(), name: "fly".into(), arguments: "{}".into() },
            ],
            usage: Usage::default(),
        };
        let mut s = setup("bad-args", vec![Ok(bad), Ok(text("ok"))], 10);
        s.agent.run("go");
        let second = &s.seen.borrow()[1];
        assert!(second[3]["content"].as_str().unwrap().contains("not valid JSON"));
        assert!(second[4]["content"].as_str().unwrap().contains("no tool named `fly`"));
    }

    #[test]
    fn a_dropped_connection_is_retried_and_a_fatal_error_is_not() {
        let mut s = setup("retry", vec![Err(ModelError::Retryable("503".into())), Ok(text("fine"))], 10);
        assert_eq!(s.agent.run("go").answer, "fine");

        let mut s = setup("fatal", vec![Err(ModelError::Fatal("401 bad key".into())), Ok(text("never"))], 10);
        assert_eq!(s.agent.run("go").outcome, Outcome::Failed("401 bad key".into()));
        assert_eq!(s.seen.borrow().len(), 1);
    }

    #[test]
    fn the_turn_limit_ends_with_a_summary_made_without_tools() {
        let again = || Ok(call("list_dir", json!({ "path": "." })));
        let mut s = setup("max-turns", vec![again(), again(), Ok(text("Listed twice; nothing else done."))], 2);
        let result = s.agent.run("go");
        assert_eq!(result.outcome, Outcome::MaxTurns);
        assert_eq!(result.answer, "Listed twice; nothing else done.");
        let last = &s.seen.borrow()[2];
        assert!(last.last().unwrap()["content"].as_str().unwrap().contains("turn limit"));
    }

    #[test]
    fn the_same_call_over_and_over_is_warned_then_stopped() {
        let replies = (0..10).map(|_| Ok(call("read_file", json!({ "path": "missing.txt" })))).collect();
        let mut s = setup("repeat", replies, 30);
        let result = s.agent.run("go");
        assert_eq!(result.outcome, Outcome::Loop);
        assert_eq!(result.turns, LOOP_STOP);
        let third_result = &s.seen.borrow()[3][7]["content"];
        assert!(third_result.as_str().unwrap().contains("this exact call 3 times"));
    }

    #[test]
    fn an_empty_reply_gets_one_prompt_to_continue() {
        let mut s = setup("empty", vec![Ok(text("")), Ok(text("All done."))], 10);
        let result = s.agent.run("go");
        assert_eq!(result.answer, "All done.");
        assert!(s.seen.borrow()[1].last().unwrap()["content"].as_str().unwrap().contains("empty reply"));
    }

    #[test]
    fn old_tool_output_is_dropped_when_the_conversation_outgrows_the_window() {
        let big = "x".repeat(4_000);
        let mut messages = vec![json!({ "role": "system", "content": "sys" })];
        for i in 0..20 {
            messages.push(json!({ "role": "tool", "tool_call_id": format!("c{i}"), "content": big }));
        }
        let dropped = trim_context(&mut messages, 30_000);
        assert!(dropped >= 5, "dropped {dropped}");
        assert_eq!(messages[1]["content"], DROPPED);
        assert_eq!(messages[20]["content"].as_str().unwrap().len(), 4_000, "recent output is kept");
        assert_eq!(trim_context(&mut messages, 1_000_000), 0);
    }

    #[test]
    fn the_prompt_names_the_folder_and_carries_project_notes() {
        let p = system_prompt(Path::new("/work/app"), true, Some("Run `make check` before finishing.\n"));
        assert!(p.contains("The project folder is /work/app."));
        assert!(p.contains("READ-ONLY mode"));
        assert!(p.ends_with("Run `make check` before finishing."));
        assert!(!system_prompt(Path::new("/w"), false, None).contains("READ-ONLY"));
    }

    #[test]
    fn a_stop_request_ends_the_run_and_answers_every_call() {
        let mut s = setup(
            "stop",
            vec![Ok(calls(vec![("list_dir", json!({ "path": "." })), ("list_dir", json!({ "path": "." }))])), Ok(text("never"))],
            10,
        );
        // Ctrl-C arrives while the first tool is running.
        let stop = s.agent.stop.clone();
        s.agent.on_event(move |event| {
            if matches!(event, Event::ToolFinished { .. }) {
                stop.request();
            }
        });
        let result = s.agent.run("look");
        assert_eq!(result.outcome, Outcome::Stopped);
        assert_eq!(s.seen.borrow().len(), 1, "no model call after the stop");
        let tool_results: Vec<&Value> = s.agent.messages.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(tool_results.len(), 2, "each call needs its result or the next request is malformed");
        assert!(tool_results[1]["content"].as_str().unwrap().contains("stopped"));
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn streamed_text_arrives_as_deltas_and_clear_forgets_the_conversation() {
        let mut s = setup("stream", vec![Ok(text("All done."))], 5);
        s.agent.stream = true;
        let heard = Rc::new(RefCell::new(String::new()));
        let sink = heard.clone();
        s.agent.on_event(move |event| {
            if let Event::Delta(piece) = event {
                sink.borrow_mut().push_str(piece);
            }
        });
        let result = s.agent.run("go");
        assert_eq!(result.answer, "All done.");
        assert_eq!(*heard.borrow(), "All done.");
        assert_eq!(s.agent.last_prompt_tokens, 10);
        s.agent.clear();
        assert_eq!(s.agent.messages.len(), 1);
        let _ = std::fs::remove_dir_all(&s.dir);
    }
}
