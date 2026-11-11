//! Claude through Anthropic's own Messages API (`POST /v1/messages`), over
//! raw HTTP: Rust has no official Anthropic SDK.
//!
//! Why not Anthropic's OpenAI-compatible endpoint: prompt caching. The agent
//! resends the whole conversation every turn; with caching, the part sent
//! before is read back at about a tenth of the price, or less.
//!
//! - **Caching.** One marker on the system prompt (tools and system render
//!   before it, so both are cached together), plus the request's top-level
//!   automatic marker, which moves with the end of the conversation. A turn
//!   adds two positions (the tool calls, then their results), well inside the
//!   API's 20-position lookback, so each request reads what the last wrote.
//! - **Thinking blocks** are kept with the assistant message (`native`) and
//!   passed back unchanged, as the API requires.
//! - **Refusals.** Models that can decline a request get the server-side
//!   fallback (`fallbacks: "default"`): on a decline the API runs the request
//!   on the model it recommends, in the same call. A decline with no fallback
//!   ends the request with the reason.
//!
//! The agent keeps its conversation in chat-completions shape; `to_messages`
//! turns it into this API's shape on every request.

use crate::interrupt::Stop;
use crate::provider::{Model, ModelError, Reply, ToolCall, Usage, STOPPED};
use crate::text::one_line;
use serde_json::{json, Value};
use std::io::BufRead;
use std::time::Duration;

const VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Streaming, so a large ceiling costs nothing in time-outs.
const DEFAULT_MAX_TOKENS: u64 = 64_000;

pub struct Anthropic {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Sent as `output_config.effort`: low, medium, high, xhigh or max.
    pub effort: Option<String>,
    pub max_tokens: Option<u64>,
    pub stop: Stop,
    http: ureq::Agent,
}

impl Anthropic {
    pub fn new(base_url: &str, api_key: String, model: &str) -> Self {
        let http = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(30)).timeout_read(Duration::from_secs(900)).build();
        // "https://api.anthropic.com/v1" and "https://api.anthropic.com" both work.
        let base_url = base_url.trim_end_matches('/').trim_end_matches("/v1").to_string();
        Anthropic { base_url, api_key, model: model.to_string(), effort: None, max_tokens: None, stop: Stop::new(), http }
    }

    /// Whether this model can decline a request and has a server-side fallback.
    fn has_fallback(&self) -> bool {
        ["claude-fable-5-2", "claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"].contains(&self.model.as_str())
    }

    /// Pure — unit-tested.
    pub fn request_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        let (system, messages) = to_messages(messages);
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "stream": true,
            "messages": messages,
            // Automatic caching: the marker follows the end of the conversation.
            "cache_control": { "type": "ephemeral" },
        });
        if let Some(system) = system {
            body["system"] = json!([{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }]);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.iter().map(to_tool).collect());
        }
        if let Some(effort) = &self.effort {
            body["output_config"] = json!({ "effort": effort });
        }
        if self.has_fallback() {
            body["fallbacks"] = json!("default");
        }
        body
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(20))
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", VERSION)
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, response) => format!("{url} answered {code}: {}", error_text(response)),
                ureq::Error::Transport(t) => format!("could not reach {url}: {t}"),
            })?;
        response.into_json().map_err(|e| format!("unreadable answer from {url}: {e}"))
    }
}

impl Model for Anthropic {
    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError> {
        self.chat_stream(messages, tools, &mut |_| {})
    }

    fn chat_stream(&mut self, messages: &[Value], tools: &[Value], on_text: &mut dyn FnMut(&str)) -> Result<Reply, ModelError> {
        let url = format!("{}/v1/messages", self.base_url);
        let mut request = self
            .http
            .post(&url)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", VERSION)
            .set("content-type", "application/json");
        if self.has_fallback() {
            request = request.set("anthropic-beta", FALLBACK_BETA);
        }
        let response = match request.send_json(self.request_body(messages, tools)) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                let message = format!("the Anthropic API answered {code}: {}", error_text(response));
                // 529 is "overloaded".
                return Err(if code == 408 || code == 429 || code >= 500 { ModelError::Retryable(message) } else { ModelError::Fatal(message) });
            }
            Err(ureq::Error::Transport(t)) => return Err(ModelError::Retryable(format!("could not reach {url}: {t}"))),
        };
        let mut lines = std::io::BufReader::new(response.into_reader());
        let mut stream = Stream::default();
        let mut line = String::new();
        loop {
            if self.stop.requested() {
                return Err(ModelError::Fatal(STOPPED.to_string()));
            }
            line.clear();
            match lines.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Some(text) = stream.push_line(&line)? {
                        on_text(&text);
                    }
                    if stream.finished {
                        break;
                    }
                }
                // Text already shown cannot be taken back: a retry would show it twice.
                Err(e) if !stream.shown_text => return Err(ModelError::Retryable(format!("the answer stopped arriving: {e}"))),
                Err(e) => return Err(ModelError::Fatal(format!("the answer stopped arriving part way: {e}"))),
            }
        }
        stream.finish()
    }

    fn list_models(&self) -> Result<Vec<String>, String> {
        let body = self.get("/v1/models?limit=1000")?;
        let mut ids: Vec<String> = body["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
        ids.sort();
        Ok(ids)
    }

    fn context_window(&self) -> Option<usize> {
        let body = self.get(&format!("/v1/models/{}", self.model)).ok()?;
        body["max_input_tokens"].as_u64().map(|n| n as usize)
    }
}

/// The message in an error body, or the body itself.
fn error_text(response: ureq::Response) -> String {
    let text = response.into_string().unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string));
    one_line(&message.unwrap_or(text), 300)
}

/// Pure — unit-tested. A chat-completions tool definition → this API's.
/// `eager_input_streaming` streams a tool's input as it is written rather than
/// all at once at the end; the agent checks it parses before running the tool.
pub fn to_tool(definition: &Value) -> Value {
    let function = &definition["function"];
    json!({
        "name": function["name"],
        "description": function["description"],
        "input_schema": function["parameters"],
        "eager_input_streaming": true,
    })
}

/// Pure — unit-tested. The agent's conversation (chat-completions shape) →
/// (system prompt, messages in this API's shape).
///
/// - Tool results become `tool_result` blocks in a user message, and messages
///   of the same role in a row are merged: the API wants them alternating,
///   with all of one turn's results in one message.
/// - An assistant message with a `native` form is sent as that, unchanged.
/// - Empty messages are left out.
pub fn to_messages(messages: &[Value]) -> (Option<String>, Vec<Value>) {
    let mut system: Option<String> = None;
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        let role = message["role"].as_str().unwrap_or("");
        let text = message["content"].as_str().unwrap_or("");
        let (role, blocks): (&str, Vec<Value>) = match role {
            "system" if i == 0 => {
                system = Some(text.to_string());
                continue;
            }
            "system" | "user" => ("user", text_block(text)),
            "tool" => {
                let content = if text.is_empty() { "(no output)" } else { text };
                ("user", vec![json!({ "type": "tool_result", "tool_use_id": message["tool_call_id"], "content": content })])
            }
            "assistant" => match message.get("native").and_then(Value::as_array) {
                Some(native) => ("assistant", native.clone()),
                None => {
                    let mut blocks = text_block(text);
                    for call in message["tool_calls"].as_array().into_iter().flatten() {
                        let arguments = call["function"]["arguments"].as_str().unwrap_or("{}");
                        let input = serde_json::from_str::<Value>(arguments).ok().filter(Value::is_object).unwrap_or_else(|| json!({}));
                        blocks.push(json!({ "type": "tool_use", "id": call["id"], "name": call["function"]["name"], "input": input }));
                    }
                    ("assistant", blocks)
                }
            },
            _ => continue,
        };
        if blocks.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((last_role, last_blocks)) if last_role == role => last_blocks.extend(blocks),
            _ => out.push((role.to_string(), blocks)),
        }
    }
    let messages = out.into_iter().map(|(role, content)| json!({ "role": role, "content": content })).collect();
    (system, messages)
}

fn text_block(text: &str) -> Vec<Value> {
    if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![json!({ "type": "text", "text": text })]
    }
}

/// A streamed reply being put together from its events.
#[derive(Default)]
pub struct Stream {
    /// The reply's content blocks as they arrive, by index.
    blocks: Vec<Value>,
    usage: Usage,
    stop_reason: Option<String>,
    stop_details: Value,
    /// Answer text has been handed to the screen.
    pub shown_text: bool,
    /// `message_stop` arrived.
    pub finished: bool,
}

impl Stream {
    /// Pure — unit-tested. One line of the event stream; returns the answer
    /// text it carried, if any.
    pub fn push_line(&mut self, line: &str) -> Result<Option<String>, ModelError> {
        let Some(data) = line.trim().strip_prefix("data:") else { return Ok(None) }; // "event:" lines, blank lines
        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else { return Ok(None) };
        match event["type"].as_str().unwrap_or("") {
            "message_start" => {
                let usage = &event["message"]["usage"];
                let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                // `input_tokens` is only the uncached part; the whole prompt is the sum.
                self.usage.input = usage["input_tokens"].as_u64().unwrap_or(0) + read + write;
                self.usage.cached = read;
                self.usage.cache_write = write;
                self.usage.output = usage["output_tokens"].as_u64().unwrap_or(0);
            }
            "content_block_start" => {
                let index = event["index"].as_u64().unwrap_or(self.blocks.len() as u64) as usize;
                while self.blocks.len() <= index {
                    self.blocks.push(Value::Null);
                }
                self.blocks[index] = event["content_block"].clone();
            }
            "content_block_delta" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let delta = &event["delta"];
                let Some(block) = self.blocks.get_mut(index) else { return Ok(None) };
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let piece = delta["text"].as_str().unwrap_or("");
                        append(block, "text", piece);
                        if !piece.is_empty() {
                            self.shown_text = true;
                            return Ok(Some(piece.to_string()));
                        }
                    }
                    "thinking_delta" => append(block, "thinking", delta["thinking"].as_str().unwrap_or("")),
                    "signature_delta" => block["signature"] = delta["signature"].clone(),
                    // Kept on the block while it arrives; removed in `finish`.
                    "input_json_delta" => append(block, "_partial", delta["partial_json"].as_str().unwrap_or("")),
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                if !event["delta"]["stop_details"].is_null() {
                    self.stop_details = event["delta"]["stop_details"].clone();
                }
                if let Some(output) = event["usage"]["output_tokens"].as_u64() {
                    self.usage.output = output;
                }
            }
            "message_stop" => self.finished = true,
            "error" => {
                let message = event["error"]["message"].as_str().unwrap_or("unknown error").to_string();
                let kind = event["error"]["type"].as_str().unwrap_or("");
                let retry = matches!(kind, "overloaded_error" | "api_error" | "rate_limit_error") && !self.shown_text;
                let message = format!("the Anthropic API reported an error: {message}");
                return Err(if retry { ModelError::Retryable(message) } else { ModelError::Fatal(message) });
            }
            _ => {} // ping, and event types added later
        }
        Ok(None)
    }

    /// The finished reply. A refusal with no fallback that rescued it is an
    /// error: the partial answer is not an answer.
    pub fn finish(self) -> Result<Reply, ModelError> {
        if self.stop_reason.as_deref() == Some("refusal") {
            let category = self.stop_details["category"].as_str().map(|c| format!(" (category: {c})")).unwrap_or_default();
            return Err(ModelError::Fatal(format!("the model declined this request{category}")));
        }
        // Tool input arrives as JSON in pieces; the whole is the block's input.
        // Input that does not parse (cut off at max_tokens) goes to the agent
        // as it came, which tells the model it was not valid JSON.
        let mut blocks: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        for block in blocks.iter_mut().filter(|b| b["type"] == "tool_use") {
            let text = block.as_object_mut().and_then(|o| o.remove("_partial")).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let parsed = if text.trim().is_empty() { Some(json!({})) } else { serde_json::from_str::<Value>(&text).ok().filter(Value::is_object) };
            match parsed {
                Some(input) => block["input"] = input,
                None => {
                    block["input"] = json!({});
                    block["_raw"] = json!(text);
                }
            }
        }
        let native = echo_form(blocks);

        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let last_fallback = native.iter().rposition(|b| b["type"] == "fallback");
        for (i, block) in native.iter().enumerate() {
            match block["type"].as_str().unwrap_or("") {
                "text" => content.push_str(block["text"].as_str().unwrap_or("")),
                // Only the model that answered in the end called tools.
                "tool_use" if last_fallback.is_none_or(|f| i > f) => {
                    let arguments = block["_raw"].as_str().map(str::to_string).unwrap_or_else(|| block["input"].to_string());
                    tool_calls.push(ToolCall { id: block["id"].as_str().unwrap_or("").to_string(), name: block["name"].as_str().unwrap_or("").to_string(), arguments });
                }
                _ => {}
            }
        }
        let native: Vec<Value> = native
            .into_iter()
            .map(|mut b| {
                b.as_object_mut().map(|o| o.remove("_raw"));
                b
            })
            .collect();
        Ok(Reply { content, tool_calls, usage: self.usage, native: Some(Value::Array(native)) })
    }
}

fn append(block: &mut Value, key: &str, piece: &str) {
    let so_far = block[key].as_str().unwrap_or("").to_string();
    block[key] = json!(so_far + piece);
}

/// Pure — unit-tested. The blocks as they must be sent back. After a
/// fallback part way through, thinking, tool_use and unknown blocks from
/// before the last `fallback` block are left out (text, server-tool pairs
/// and the `fallback` block itself stay where they were).
pub fn echo_form(blocks: Vec<Value>) -> Vec<Value> {
    let last_fallback = blocks.iter().rposition(|b| b["type"] == "fallback");
    let mut out = Vec::new();
    for (i, block) in blocks.into_iter().enumerate() {
        let kind = block["type"].as_str().unwrap_or("").to_string();
        let before_boundary = last_fallback.is_some_and(|f| i < f);
        let keep = !before_boundary || matches!(kind.as_str(), "text" | "server_tool_use" | "fallback") || kind.ends_with("_tool_result");
        if !keep {
            continue;
        }
        out.push(block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation() -> Vec<Value> {
        vec![
            json!({ "role": "system", "content": "You are rak." }),
            json!({ "role": "user", "content": "Fix it." }),
            json!({ "role": "assistant", "content": "Looking.", "tool_calls": [
                { "id": "t1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.py\"}" } },
                { "id": "t2", "type": "function", "function": { "name": "list_dir", "arguments": "not json" } } ] }),
            json!({ "role": "tool", "tool_call_id": "t1", "content": "print(1)" }),
            json!({ "role": "tool", "tool_call_id": "t2", "content": "" }),
            json!({ "role": "user", "content": "[progress note] finish up." }),
            json!({ "role": "assistant", "content": "" }),
            json!({ "role": "user", "content": "Continue." }),
        ]
    }

    #[test]
    fn the_conversation_is_turned_into_alternating_messages() {
        let (system, messages) = to_messages(&conversation());
        assert_eq!(system.as_deref(), Some("You are rak."));
        let roles: Vec<&str> = messages.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user"], "the empty assistant reply is dropped and the users around it merged");
        let assistant = &messages[1]["content"];
        assert_eq!(assistant[0], json!({ "type": "text", "text": "Looking." }));
        assert_eq!(assistant[1]["input"], json!({ "path": "a.py" }));
        assert_eq!(assistant[2]["input"], json!({}), "unparseable arguments are sent as an empty object");
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results[0], json!({ "type": "tool_result", "tool_use_id": "t1", "content": "print(1)" }));
        assert_eq!(results[1]["content"], "(no output)");
        assert_eq!(results[2]["type"], "text", "the note comes after the results, in the same message");
        assert_eq!(results[3]["text"], "Continue.");
    }

    #[test]
    fn a_native_reply_is_sent_back_unchanged() {
        let native = json!([{ "type": "thinking", "thinking": "", "signature": "sig" }, { "type": "text", "text": "Done." }]);
        let messages = vec![json!({ "role": "user", "content": "Go." }), json!({ "role": "assistant", "content": "Done.", "native": native })];
        let (_, out) = to_messages(&messages);
        assert_eq!(out[1]["content"], native);
    }

    #[test]
    fn the_request_caches_the_prefix_and_asks_for_a_fallback_only_where_one_exists() {
        let tool = json!({ "type": "function", "function": { "name": "read_file", "description": "Reads.", "parameters": { "type": "object", "properties": {} } } });
        let mut p = Anthropic::new("https://api.anthropic.com/v1/", "k".into(), "claude-opus-5-5");
        assert_eq!(p.base_url, "https://api.anthropic.com");
        p.effort = Some("high".into());
        let body = p.request_body(&conversation(), &[tool]);
        assert_eq!(body["cache_control"], json!({ "type": "ephemeral" }));
        assert_eq!(body["system"][0]["cache_control"], json!({ "type": "ephemeral" }));
        assert_eq!(body["system"][0]["text"], "You are rak.");
        assert_eq!(body["tools"][0], json!({ "name": "read_file", "description": "Reads.", "input_schema": { "type": "object", "properties": {} }, "eager_input_streaming": true }));
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["max_tokens"], 64_000);
        assert!(body.get("thinking").is_none() && body.get("temperature").is_none());
        // At most four cache markers: here two (system, and the automatic one).
        assert_eq!(body.to_string().matches("cache_control").count(), 2);

        let haiku = Anthropic::new(DEFAULT_BASE_URL, "k".into(), "claude-haiku-5-5");
        assert!(haiku.request_body(&conversation(), &[]).get("fallbacks").is_none());
    }

    fn events(lines: &[Value]) -> Stream {
        let mut s = Stream::default();
        for line in lines {
            s.push_line(&format!("data: {line}")).unwrap();
        }
        s
    }

    #[test]
    fn a_streamed_reply_is_put_together_with_its_usage() {
        let mut s = Stream::default();
        assert_eq!(s.push_line("event: message_start").unwrap(), None);
        s.push_line(&format!("data: {}", json!({ "type": "message_start", "message": { "usage": { "input_tokens": 40, "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 200, "output_tokens": 1 } } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "" } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "abc" } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text", "text": "" } }))).unwrap();
        let piece = s.push_line(&format!("data: {}", json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "Reading." } }))).unwrap();
        assert_eq!(piece.as_deref(), Some("Reading."));
        s.push_line(&format!("data: {}", json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {} } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"pa" } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "th\": \"a.py\"}" } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 57 } }))).unwrap();
        s.push_line(&format!("data: {}", json!({ "type": "message_stop" }))).unwrap();
        assert!(s.finished);
        let reply = s.finish().unwrap();
        assert_eq!(reply.content, "Reading.");
        assert_eq!(reply.tool_calls, [ToolCall { id: "toolu_1".into(), name: "read_file".into(), arguments: "{\"path\":\"a.py\"}".into() }]);
        assert_eq!(reply.usage, Usage { input: 3240, output: 57, cached: 3000, cache_write: 200 });
        let native = reply.native.unwrap();
        assert_eq!(native[0], json!({ "type": "thinking", "thinking": "", "signature": "abc" }), "thinking kept to send back");
        assert_eq!(native[2]["input"], json!({ "path": "a.py" }));
        assert!(native[2].get("_partial").is_none() && native[2].get("_raw").is_none());
    }

    #[test]
    fn tool_input_cut_off_reaches_the_agent_as_it_came() {
        let s = events(&[
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "tool_use", "id": "t", "name": "write_file", "input": {} } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "input_json_delta", "partial_json": "{\"path\": \"a.py\", \"content\": \"def" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "max_tokens" }, "usage": { "output_tokens": 9 } }),
        ]);
        let reply = s.finish().unwrap();
        assert_eq!(reply.tool_calls[0].arguments, "{\"path\": \"a.py\", \"content\": \"def");
        assert_eq!(reply.native.unwrap()[0]["input"], json!({}));
    }

    #[test]
    fn a_refusal_is_an_error_and_a_fallback_part_way_drops_what_came_before() {
        let refused = events(&[json!({ "type": "message_delta", "delta": { "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": "cyber" } } })]);
        let Err(ModelError::Fatal(why)) = refused.finish() else { panic!() };
        assert!(why.contains("declined") && why.contains("cyber"), "{why}");

        let rescued = events(&[
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "", "signature": "s1" } }),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text", "text": "Part one " } }),
            json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "early", "name": "run_shell", "input": {} } }),
            json!({ "type": "content_block_start", "index": 3, "content_block": { "type": "fallback", "from": { "model": "claude-opus-5-5" }, "to": { "model": "claude-opus-5" } } }),
            json!({ "type": "content_block_start", "index": 4, "content_block": { "type": "text", "text": "and part two." } }),
            json!({ "type": "content_block_start", "index": 5, "content_block": { "type": "tool_use", "id": "late", "name": "read_file", "input": {} } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
        ]);
        let reply = rescued.finish().unwrap();
        assert_eq!(reply.content, "Part one and part two.");
        let ids: Vec<&str> = reply.tool_calls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["late"], "only the model that answered runs tools");
        let kinds: Vec<String> = reply.native.unwrap().as_array().unwrap().iter().map(|b| b["type"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["text", "fallback", "text", "tool_use"], "thinking and tool_use before the boundary are not echoed");
    }

    #[test]
    fn errors_in_the_stream_are_retried_only_before_text_is_shown() {
        let overloaded = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });
        assert!(matches!(Stream::default().push_line(&format!("data: {overloaded}")), Err(ModelError::Retryable(_))));
        let mut started = Stream::default();
        started.push_line(&format!("data: {}", json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }))).unwrap();
        started.push_line(&format!("data: {}", json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Hi" } }))).unwrap();
        assert!(matches!(started.push_line(&format!("data: {overloaded}")), Err(ModelError::Fatal(_))));
    }
}
