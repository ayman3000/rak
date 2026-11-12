//! The agent loop: ask the model, run the tools it calls, give it the results,
//! repeat until it answers without calling a tool.

use crate::compact;
use crate::interrupt::Stop;
use crate::provider::{Model, ModelError, Reply, Usage};
use crate::tools::{definition, Image, Tool};
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
    Refused {
        tool: String,
    },
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
    ModelCalled {
        turn: usize,
        usage: Usage,
        millis: u128,
    },
    Retrying {
        attempt: usize,
        error: &'a str,
    },
    /// A piece of the model's text as it arrives (only when `stream` is on).
    Delta(&'a str),
    /// Text the model wrote alongside tool calls, whole.
    Text(&'a str),
    ToolStarted {
        name: &'a str,
        args: &'a Value,
    },
    ToolFinished {
        name: &'a str,
        ok: bool,
        denied: bool,
        text: &'a str,
        millis: u128,
    },
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
/// Characters counted as one token when sizing the conversation against the
/// window. Low on purpose (code runs nearer 3.5–4): better early than over.
const CHARS_PER_TOKEN: usize = 3;
const DROPPED: &str = "[earlier output removed to save context; run the tool again if you need it]";

/// Asked before a tool that changes something runs.
type ApproveFn = Box<dyn FnMut(&str, &Value) -> Approval>;

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
    /// What the model is told when the user says no to a tool call.
    /// `REFUSAL_NOTE` unless changed (rak adds a hint to keep the lesson).
    pub refusal_note: String,
    /// Asked when the model gives its final answer: a note sends it back to
    /// work (rak: steps still open in the task list). Should say something
    /// only once per request, or the model never gets to finish.
    pub finish_check: Option<Box<dyn FnMut() -> Option<String>>>,
    approve: ApproveFn,
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
            refusal_note: REFUSAL_NOTE.to_string(),
            finish_check: None,
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

    pub fn tool_names(&self) -> Vec<&str> {
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

    /// The conversation without its system prompt: what a saved chat keeps.
    pub fn history(&self) -> &[Value] {
        &self.messages[1..]
    }

    /// Picks up a saved conversation under this agent's system prompt.
    pub fn restore(&mut self, history: Vec<Value>) {
        self.clear();
        self.messages.extend(history.into_iter().filter(|m| m["role"] != "system"));
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
        // The tool descriptions go with every call, so they come out of the budget.
        let definitions_size: usize = definitions.iter().map(|d| d.to_string().len()).sum();
        let budget = (self.config.context_window * CHARS_PER_TOKEN).saturating_sub(definitions_size).max(1);

        for turn in 1..=self.config.max_turns {
            self.fit(budget);
            if self.stop.requested() {
                return self.stopped(turn);
            }
            let mut overflow_handled = false;
            let reply = loop {
                match self.call_model(turn, &definitions) {
                    Ok(r) => break r,
                    Err(_) if self.stop.requested() => return self.stopped(turn),
                    // The server counts tokens its own way: when it says the
                    // request is too long, shrink harder and ask once more.
                    Err(e) if compact::is_overflow(&e) && !overflow_handled => {
                        overflow_handled = true;
                        (self.on_event)(&Event::Note("the model says the conversation is too long for it"));
                        // Our estimate said it fits, so shrink by what is there, not by the budget.
                        let size = conversation_size(&self.messages);
                        if self.compact(size / 4) || trim_keeping(&mut self.messages, size / 2, 0) > 0 {
                            continue;
                        }
                        let why = format!(
                            "{e}\nThe conversation is too long for the model even with its earlier part summarized: \
                             the latest message or tool output alone is too big."
                        );
                        return RunResult { outcome: Outcome::Failed(why), answer: String::new(), turns: turn };
                    }
                    Err(e) => return RunResult { outcome: Outcome::Failed(e), answer: String::new(), turns: turn },
                }
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
                // The final answer keeps its native form too: Anthropic wants
                // thinking blocks passed back unchanged on every request.
                let mut message = json!({ "role": "assistant", "content": reply.content });
                if let Some(native) = &reply.native {
                    message["native"] = native.clone();
                }
                self.messages.push(message);
                // Work left undone (an open task list): asked once to finish it.
                if let Some(note) = self.finish_check.as_mut().and_then(|check| check()) {
                    (self.on_event)(&Event::Note("the task list has open steps: asked to finish them"));
                    self.messages.push(json!({ "role": "user", "content": note }));
                    continue;
                }
                return RunResult { outcome: Outcome::Done, answer: reply.content, turns: turn };
            }

            self.messages.push(assistant_message(&reply));
            if !reply.content.trim().is_empty() {
                (self.on_event)(&Event::Text(&reply.content));
            }

            let mut refused: Option<String> = None;
            let mut images: Vec<Image> = Vec::new();
            for call in &reply.tool_calls {
                let mut result = if refused.is_some() {
                    (false, self.refusal_note.clone())
                } else if self.stop.requested() {
                    (false, STOP_NOTE.to_string())
                } else {
                    let (ok, text, image) = self.run_tool(&call.name, &call.arguments, &mut refused);
                    images.extend(image);
                    (ok, text)
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
            // A tool result holds text only, so images come right after the
            // turn's results, in one message the model reads with them.
            if !images.is_empty() {
                self.messages.push(image_message(&images));
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

    /// Brings the conversation under `budget` characters, cheapest step first:
    /// old tool outputs become one-line stubs; if that is not enough, the older
    /// part is summarized; if even that is not enough, recent tool outputs are
    /// stubbed too, oldest first.
    fn fit(&mut self, budget: usize) {
        let dropped = trim_context(&mut self.messages, budget);
        if dropped > 0 {
            (self.on_event)(&Event::Note(&format!("context is filling up: dropped {dropped} old tool outputs")));
        }
        if conversation_size(&self.messages) > budget {
            self.compact(budget / 4);
        }
        if conversation_size(&self.messages) > budget {
            let dropped = trim_keeping(&mut self.messages, budget, 0);
            if dropped > 0 {
                (self.on_event)(&Event::Note(&format!("context is still full: dropped {dropped} recent tool outputs")));
            }
        }
    }

    /// Replaces the older part of the conversation with a summary the model
    /// writes, keeping about `tail_budget` characters of the newest turns word
    /// for word. False when there is nothing older to summarize.
    pub fn compact(&mut self, tail_budget: usize) -> bool {
        let (middle, tail) = compact::split(&self.messages[1..], tail_budget, message_size);
        if middle.is_empty() {
            return false;
        }
        (self.on_event)(&Event::Note(&format!("context is full: summarizing {} earlier messages", middle.len())));
        // Half the window for what the summary is written from: room for the reply.
        let source = compact::transcript(&middle, (self.config.context_window * CHARS_PER_TOKEN / 2).max(4_000));
        let request = [json!({ "role": "system", "content": compact::SUMMARY_PROMPT }), json!({ "role": "user", "content": source })];
        let mut summary = None;
        for attempt in 0..2 {
            match self.model.chat(&request, &[]) {
                Ok(reply) => {
                    self.usage.add(reply.usage);
                    summary = Some(reply.content).filter(|s| !s.trim().is_empty());
                    break;
                }
                Err(ModelError::Retryable(_)) if attempt == 0 && !self.stop.requested() => std::thread::sleep(self.retry_base),
                Err(_) => break,
            }
        }
        let summary = summary.unwrap_or_else(|| {
            (self.on_event)(&Event::Note("no summary could be written: kept a list of the requests and calls instead"));
            compact::plain_summary(&middle)
        });
        let checkpoint = compact::checkpoint(&summary, compact::latest_task_list(&middle).as_deref());
        self.messages.truncate(1);
        self.messages.extend(compact::assemble(&checkpoint, tail));
        self.last_prompt_tokens = 0;
        true
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
                    self.usage.add(reply.usage);
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

    /// Runs one tool call and returns (ok, text for the model, an image if it
    /// brought one). Sets `refused` when the person said no.
    fn run_tool(&mut self, name: &str, arguments: &str, refused: &mut Option<String>) -> (bool, String, Option<Image>) {
        let Some(tool) = self.tools.iter().find(|t| t.name() == name) else {
            return (false, format!("There is no tool named `{name}`. The tools are: {}.", self.tool_names().join(", ")), None);
        };
        let args: Value = match serde_json::from_str(if arguments.trim().is_empty() { "{}" } else { arguments }) {
            Ok(v @ Value::Object(_)) => v,
            Ok(_) => return (false, format!("The arguments for {name} must be a JSON object."), None),
            Err(e) => return (false, format!("The arguments for {name} were not valid JSON ({e}). Send one JSON object."), None),
        };
        self.tool_calls += 1;
        (self.on_event)(&Event::ToolStarted { name, args: &args });

        if tool.needs_approval() && (self.approve)(name, &args) == Approval::No {
            *refused = Some(name.to_string());
            (self.on_event)(&Event::ToolFinished { name, ok: false, denied: true, text: &self.refusal_note, millis: 0 });
            return (false, self.refusal_note.clone(), None);
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
        (result.ok, result.text, result.image)
    }
}

/// The message that shows the model the images a turn's tools brought, in
/// chat-completions content parts. Providers with their own form convert it.
pub fn image_message(images: &[Image]) -> Value {
    let mut parts = vec![json!({ "type": "text", "text": "The image you asked to view:" })];
    if images.len() > 1 {
        parts[0] = json!({ "type": "text", "text": "The images you asked to view, in order:" });
    }
    parts.extend(images.iter().map(|image| json!({ "type": "image_url", "image_url": { "url": image.data_url() } })));
    json!({ "role": "user", "content": parts })
}

/// About what an image costs in context, counted in characters like text
/// (roughly 1,600 tokens), not by the size of its base64.
const IMAGE_CHARS: usize = 5_000;
const IMAGE_DROPPED: &str = "[an earlier image was removed to save context; view it again if you need it]";

/// A message's size for the context budget: images at a flat rate.
fn message_size(message: &Value) -> usize {
    match message["content"].as_array() {
        Some(parts) => {
            100 + parts
                .iter()
                .map(|p| if p["type"] == "image_url" { IMAGE_CHARS } else { p["text"].as_str().map_or(50, str::len) })
                .sum::<usize>()
        }
        None => message.to_string().len(),
    }
}

fn assistant_message(reply: &Reply) -> Value {
    let calls: Vec<Value> = reply
        .tool_calls
        .iter()
        .map(|c| json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments } }))
        .collect();
    let content = if reply.content.is_empty() { Value::Null } else { json!(reply.content) };
    let mut message = json!({ "role": "assistant", "content": content, "tool_calls": calls });
    if let Some(native) = &reply.native {
        message["native"] = native.clone();
    }
    message
}

/// Pure — unit-tested. What replaces a removed output: which call it was and
/// how it began, so the model knows what it no longer has and can get it back.
pub fn dropped_stub(call: Option<&(String, String)>, output: &str) -> String {
    let Some((name, target)) = call else { return DROPPED.to_string() };
    let first = output.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    format!(
        "[output of {name} {target} removed to save context. It began: {}. Run it again if you need it.]",
        crate::text::one_line(first, 120)
    )
    .replace("  ", " ")
}

/// The conversation's size for the context budget, in characters.
pub fn conversation_size(messages: &[Value]) -> usize {
    messages.iter().map(message_size).sum()
}

/// Pure — unit-tested. When the conversation is longer than `budget_chars`,
/// the oldest tool outputs are replaced by a one-line stub, oldest first, until
/// it fits. The most recent messages are never touched. Returns how many
/// outputs were dropped.
pub fn trim_context(messages: &mut [Value], budget_chars: usize) -> usize {
    trim_keeping(messages, budget_chars, KEEP_RECENT)
}

/// `trim_context`, leaving the last `keep_recent` messages alone.
pub fn trim_keeping(messages: &mut [Value], budget_chars: usize, keep_recent: usize) -> usize {
    let size = message_size;
    let mut total: usize = messages.iter().map(size).sum();
    if total <= budget_chars {
        return 0;
    }
    // Which call each output answered, so its stub can say what was removed.
    let calls = compact::call_index(messages);
    let last_droppable = messages.len().saturating_sub(keep_recent);
    let mut dropped = 0;
    for message in messages[..last_droppable].iter_mut() {
        if total <= budget_chars {
            break;
        }
        let call = message["tool_call_id"].as_str().and_then(|id| calls.get(id)).cloned();
        // The task list is the plan: it stays.
        let is_plan = call.as_ref().is_some_and(|(name, _)| name == "todo");
        let is_big_tool_output = message["role"] == "tool"
            && !is_plan
            && message["content"].as_str().is_some_and(|c| c.len() > DROPPED.len() * 3 && !c.starts_with("[output of "));
        let has_image = message["content"].as_array().is_some_and(|parts| parts.iter().any(|p| p["type"] == "image_url"));
        if is_big_tool_output {
            let before = size(message);
            message["content"] = json!(dropped_stub(call.as_ref(), message["content"].as_str().unwrap_or("")));
            total -= before - size(message);
            dropped += 1;
        } else if has_image {
            let before = size(message);
            message["content"] = json!(IMAGE_DROPPED);
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
         Report only what happened: say you ran, tested or verified something only if a command you ran in this \
         conversation did it, and say whether it passed. A screenshot or log you have not looked at checks nothing. \
         Say plainly what you could not check, and why.\n\
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
        Reply { content: content.into(), tool_calls: vec![], usage: Usage { input: 10, output: 2, ..Default::default() }, native: None }
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
        Reply { content: String::new(), tool_calls, usage: Usage { input: 10, output: 2, ..Default::default() }, native: None }
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
        assert_eq!(s.agent.usage, Usage { input: 30, output: 6, ..Default::default() });
        assert_eq!(s.agent.tool_calls, 2);

        // The third request carries both tool results, matched to their calls.
        let third = &s.seen.borrow()[2];
        let roles: Vec<&str> = third.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "tool"]);
        assert_eq!(third[3]["tool_call_id"], "c0");
        assert_eq!(third[5]["content"], "hello");
    }

    #[test]
    fn work_left_open_sends_the_model_back_once() {
        let mut s = setup("finish-check", vec![Ok(text("Done.")), Ok(text("Now really done."))], 10);
        let mut asked = 0;
        s.agent.finish_check = Some(Box::new(move || {
            asked += 1;
            (asked == 1).then(|| "[task list] Run the tests.".to_string())
        }));
        let result = s.agent.run("fix it");
        assert_eq!(result.outcome, Outcome::Done);
        assert_eq!((result.answer.as_str(), result.turns), ("Now really done.", 2));
        let second = &s.seen.borrow()[1];
        assert_eq!(second.last().unwrap()["content"], "[task list] Run the tests.");
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
        s.agent.refusal_note.push_str(" Keep the lesson.");
        let result = s.agent.run("write a.txt");
        assert_eq!(result.outcome, Outcome::Refused { tool: "write_file".into() });
        assert!(!s.dir.join("a.txt").exists());
        assert_eq!(s.seen.borrow().len(), 1, "the model must not get another turn after a no");
        let tool_results: Vec<&Value> = s.agent.messages.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(tool_results.len(), 2);
        let note = format!("{REFUSAL_NOTE} Keep the lesson.");
        assert!(tool_results.iter().all(|m| m["content"] == note.as_str()), "both calls get the note, hint included");
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
            native: None,
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
    fn a_removed_output_names_its_call_and_the_task_list_is_kept() {
        let big = format!("fn main() {{}}\n{}", "x".repeat(4_000));
        let call = |id: &str, name: &str, args: Value| {
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": id, "type": "function",
                "function": { "name": name, "arguments": args.to_string() } }] })
        };
        let mut messages = vec![
            json!({ "role": "system", "content": "sys" }),
            call("a", "todo", json!({ "steps": [] })),
            json!({ "role": "tool", "tool_call_id": "a", "content": format!("Task list (0/1 done):\n[ ] {}", "y".repeat(4_000)) }),
            call("b", "read_file", json!({ "path": "src/main.rs" })),
            json!({ "role": "tool", "tool_call_id": "b", "content": big }),
        ];
        for i in 0..KEEP_RECENT {
            messages.push(json!({ "role": "user", "content": format!("filler {i}") }));
        }
        assert_eq!(trim_context(&mut messages, 1_000), 1);
        assert!(messages[2]["content"].as_str().unwrap().starts_with("Task list"), "the plan stays");
        assert_eq!(
            messages[4]["content"],
            "[output of read_file src/main.rs removed to save context. It began: fn main() {}. Run it again if you need it.]"
        );
        assert_eq!(dropped_stub(None, "x"), DROPPED);
    }

    fn earlier_chat() -> Vec<Value> {
        let mut history = Vec::new();
        for i in 0..4 {
            history.push(json!({ "role": "user", "content": format!("request {i}: {}", "r".repeat(2_000)) }));
            history.push(json!({ "role": "assistant", "content": format!("answer {i}: {}", "a".repeat(2_000)) }));
        }
        history
    }

    #[test]
    fn a_too_long_error_summarizes_the_older_part_and_asks_again() {
        let too_long = Err(ModelError::Fatal("the Anthropic API answered 400: prompt is too long".into()));
        let mut s = setup("overflow", vec![too_long, Ok(text("They asked for four things.")), Ok(text("done"))], 10);
        s.agent.restore(earlier_chat());
        let result = s.agent.run("request 4");
        assert_eq!((result.outcome, result.answer.as_str()), (Outcome::Done, "done"));

        let seen = s.seen.borrow();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[1][0]["content"], compact::SUMMARY_PROMPT, "the second call writes the summary");
        assert!(seen[1][1]["content"].as_str().unwrap().contains("USER: request 0"));
        let retried = &seen[2];
        assert!(retried.len() < 10, "the older part is gone: {} messages", retried.len());
        let checkpoint = retried[1]["content"].as_str().unwrap();
        assert!(checkpoint.starts_with(compact::CHECKPOINT_HEAD) && checkpoint.contains("They asked for four things."));
        assert_eq!(retried.last().unwrap()["content"], "request 4", "the latest request is kept word for word");
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn a_conversation_over_budget_is_summarized_before_the_call() {
        let mut s = setup("compact-ahead", vec![Err(ModelError::Fatal("down".into())), Ok(text("done"))], 10);
        s.agent.set_context_window(4_000);
        s.agent.restore(earlier_chat());
        let result = s.agent.run("request 4");
        assert_eq!(result.answer, "done");
        let seen = s.seen.borrow();
        // The summary call failed, so a plain list of the requests stands in.
        let checkpoint = seen[1][1]["content"].as_str().unwrap();
        assert!(checkpoint.contains("No summary could be written") && checkpoint.contains("- request 0"));
        assert!(conversation_size(&seen[1]) < conversation_size(&earlier_chat()));
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[test]
    fn a_too_long_error_with_nothing_to_shrink_fails_and_says_why() {
        let too_long = || Err(ModelError::Fatal("the model server answered 400: context_length_exceeded".into()));
        let mut s = setup("overflow-alone", vec![too_long(), too_long()], 10);
        let result = s.agent.run("one huge message");
        let Outcome::Failed(why) = result.outcome else { panic!("{:?}", result.outcome) };
        assert!(why.contains("latest message or tool output alone is too big"), "{why}");
        assert_eq!(s.seen.borrow().len(), 1, "nothing could shrink, so no second call");
        let _ = std::fs::remove_dir_all(&s.dir);
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

    #[test]
    fn images_are_counted_at_a_flat_rate_and_old_ones_are_dropped_first_when_full() {
        let image = Image { media_type: "image/png".into(), data: "A".repeat(500_000) };
        let shown = image_message(&[image]);
        assert!(message_size(&shown) < 6_000, "an image counts as about 1,600 tokens, not its base64 length");
        assert_eq!(shown["content"][1]["type"], "image_url");

        let mut messages = vec![json!({ "role": "system", "content": "sys" }), shown];
        messages.extend((0..10).map(|i| json!({ "role": "user", "content": format!("message {i}") })));
        assert_eq!(trim_context(&mut messages, 100_000), 0, "under budget: kept");
        assert_eq!(trim_context(&mut messages, 500), 1);
        assert_eq!(messages[1]["content"], json!(IMAGE_DROPPED));
    }
}
