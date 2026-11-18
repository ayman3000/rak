//! A ChatGPT plan as the model provider: OpenAI's Responses API
//! (`POST /v1/responses`) with the access token from Sign in with ChatGPT
//! (auth.rs) in place of an API key. Requests count against the person's
//! ChatGPT plan, not an API bill.
//!
//! What this route asks for (OpenAI's "preview limitations"), and so what
//! this file does:
//! - `store: false` and `stream: true` on every request, the whole
//!   conversation in `input` each time (nothing is kept on their side);
//! - the system prompt as `instructions`, never a `system` message;
//! - no `max_output_tokens`, `temperature` or `truncation`.
//!
//! **Reasoning goes back.** With nothing stored, a reasoning model's thinking
//! between tool calls is lost unless it is sent back: the reply's own output
//! items (reasoning with `encrypted_content`, text, tool calls) are kept with
//! the assistant message (`native`, under "chatgpt") and sent back unchanged.
//!
//! **A session id for the prompt cache.** Every request of one model object
//! carries the same random `session_id` header (and `prompt_cache_key`).
//! Measured on 2026-11-18: without the header the plan route cached nothing
//! of a repeated 4K-token prompt; with it, 4096 of 4238 tokens from the
//! second request on. The header is not in OpenAI's documentation for this
//! route; it names nothing but this conversation.
//!
//! The agent keeps its conversation in chat-completions shape; `to_input`
//! turns it into this API's items on every request.

pub mod auth;

use crate::interrupt::Stop;
use crate::provider::{Model, ModelError, Reply, ToolCall, Usage, STOPPED};
use crate::text::one_line;
use serde_json::{json, Value};
use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// Where a reply's own items are kept in a message's `native`.
const NATIVE: &str = "chatgpt";

pub struct ChatGpt {
    pub base_url: String,
    pub model: String,
    /// Sent as `reasoning.effort`.
    pub effort: Option<String>,
    /// A default effort, not the person's choice: on 400/422 the request is
    /// tried once more without it (see OpenAICompatible::effort_optional).
    pub effort_optional: bool,
    pub stop: Stop,
    pub stall_base: Duration,
    /// Where the sign-in is kept; None: nowhere to keep one (no home folder).
    pub account_file: Option<PathBuf>,
    /// A token to send as it is, for tests: no sign-in file is read.
    pub fixed_token: Option<String>,
    /// Random, one per model object: keeps a conversation's requests on the
    /// same prompt cache (see the top of this file).
    session: String,
    clients: crate::stall::Clients,
}

impl ChatGpt {
    pub fn new(base_url: &str, model: &str) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        ChatGpt {
            stall_base: crate::stall::base_for(&base_url, false),
            base_url,
            model: model.to_string(),
            effort: None,
            effort_optional: false,
            stop: Stop::new(),
            account_file: auth::file(),
            fixed_token: None,
            session: auth::random_token(),
            clients: Default::default(),
        }
    }

    fn token(&self, refused: Option<&str>) -> Result<String, String> {
        if let Some(token) = &self.fixed_token {
            return Ok(token.clone());
        }
        let file = self.account_file.as_ref().ok_or("no home folder to keep a ChatGPT sign-in in")?;
        auth::access_token(file, refused)
    }

    /// Pure — unit-tested.
    pub fn request_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        let (instructions, input) = to_input(messages);
        let mut body = json!({
            "model": self.model,
            "input": input,
            "store": false,
            "stream": true,
            // Thinking comes back in a form that can be sent again.
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": self.session,
        });
        if let Some(instructions) = instructions {
            body["instructions"] = json!(instructions);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.iter().map(to_tool).collect());
        }
        if let Some(effort) = &self.effort {
            body["reasoning"] = json!({ "effort": effort });
        }
        body
    }

    /// One request; a refusal comes with its HTTP status.
    fn post(&self, url: &str, body: &Value, read: Duration, token: &str) -> Result<ureq::Response, (Option<u16>, ModelError)> {
        let request = self
            .clients
            .with_read_limit(read)
            .post(url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .set("session_id", &self.session);
        match request.send_json(body.clone()) {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
                let (error_code, message) = error_parts(&body, &text);
                Err((Some(code), refusal(Some(code), error_code.as_deref(), &message)))
            }
            Err(ureq::Error::Transport(t)) => Err((None, ModelError::Retryable(format!("could not reach {url}: {t}")))),
        }
    }

    /// The request, signed in: a token the API refuses (401) is renewed once.
    fn post_signed_in(&self, url: &str, body: &Value, read: Duration) -> Result<ureq::Response, (Option<u16>, ModelError)> {
        let token = self.token(None).map_err(|why| (None, ModelError::Fatal(why)))?;
        match self.post(url, body, read, &token) {
            Err((Some(401), error)) if self.fixed_token.is_none() => match self.token(Some(&token)) {
                Ok(renewed) => self.post(url, body, read, &renewed),
                Err(_) => Err((Some(401), error)),
            },
            other => other,
        }
    }

    fn models(&self) -> Result<Value, String> {
        let url = format!("{}/models", self.base_url);
        let token = self.token(None)?;
        let response = self
            .clients
            .with_read_limit(Duration::from_secs(60))
            .get(&url)
            .timeout(Duration::from_secs(20))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, response) => {
                    let text = response.into_string().unwrap_or_default();
                    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
                    format!("{url} answered {code}: {}", error_parts(&body, &text).1)
                }
                ureq::Error::Transport(t) => format!("could not reach {url}: {t}"),
            })?;
        response.into_json().map_err(|e| format!("unreadable answer from {url}: {e}"))
    }
}

impl Model for ChatGpt {
    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError> {
        self.chat_stream(messages, tools, &mut |_| {})
    }

    fn chat_stream(&mut self, messages: &[Value], tools: &[Value], on_text: &mut dyn FnMut(&str)) -> Result<Reply, ModelError> {
        let url = format!("{}/responses", self.base_url);
        let body = self.request_body(messages, tools);
        let read = crate::stall::limit(self.stall_base, body.to_string().len());
        let optional = self.effort_optional && body.get("reasoning").is_some();
        let response = match self.post_signed_in(&url, &body, read) {
            Ok(response) => response,
            // A default effort the model doesn't take: once more without it.
            Err((Some(code), error)) if optional && crate::provider::may_refuse_a_field(code) => {
                let mut without = body;
                if let Some(fields) = without.as_object_mut() {
                    fields.remove("reasoning");
                }
                let response = self.post_signed_in(&url, &without, read).map_err(|_| error)?;
                self.effort = None;
                crate::provider::note_refused_effort(&self.base_url, &self.model);
                response
            }
            Err((_, error)) => return Err(error),
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
                Err(e) if !stream.shown_text => {
                    return Err(ModelError::Retryable(format!("the answer stopped arriving: {}", crate::stall::stopped_arriving(&e, read))))
                }
                Err(e) => {
                    return Err(ModelError::Fatal(format!(
                        "the answer stopped arriving part way: {}",
                        crate::stall::stopped_arriving(&e, read)
                    )))
                }
            }
        }
        stream.finish()
    }

    /// The models this ChatGPT account may use, in the server's own order
    /// (its first is its recommendation).
    fn list_models(&self) -> Result<Vec<String>, String> {
        Ok(model_names(&self.models()?))
    }

    fn context_window(&self) -> Option<usize> {
        context_of(&self.models().ok()?, &self.model)
    }

    /// From the account's model list (`input_modalities`), else from the name.
    fn sees_images(&self) -> Option<bool> {
        self.models().ok().and_then(|body| sees_of(&body, &self.model)).or_else(|| crate::provider::known_to_see(&self.model))
    }
}

/// Pure — unit-tested. The models to offer: those marked for listing, by
/// their slug. An answer in the API's older shape (`data[].id`) is read too.
pub fn model_names(body: &Value) -> Vec<String> {
    let listed: Vec<String> = body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str().is_none_or(|v| v == "list"))
        .filter_map(|m| m["slug"].as_str().or(m["id"].as_str()).map(str::to_string))
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    body["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()
}

/// Pure — unit-tested. A model's context window, when the list gives one.
fn context_of(body: &Value, model: &str) -> Option<usize> {
    let entry = body["models"].as_array()?.iter().find(|m| m["slug"].as_str() == Some(model))?;
    ["context_window", "max_context_window", "context_length"].iter().find_map(|name| entry[*name].as_u64()).map(|n| n as usize)
}

/// Pure — unit-tested. Whether a model takes images, when the list says.
fn sees_of(body: &Value, model: &str) -> Option<bool> {
    let entry = body["models"].as_array()?.iter().find(|m| m["slug"].as_str() == Some(model))?;
    Some(entry["input_modalities"].as_array()?.iter().any(|kind| kind == "image"))
}

/// Pure — unit-tested. The code and words of an error body: the API's
/// `{"error": {"code", "message"}}`, or `{"detail": …}` from the door before it.
fn error_parts(body: &Value, raw: &str) -> (Option<String>, String) {
    let error = &body["error"];
    let message = match error {
        Value::Object(_) => error["message"].as_str().map(str::to_string).unwrap_or_else(|| error.to_string()),
        Value::String(text) => text.clone(),
        _ => match &body["detail"] {
            Value::String(text) => text.clone(),
            Value::Null => raw.to_string(),
            other => other.to_string(),
        },
    };
    let param = error["param"].as_str().map(|p| format!(" (in `{p}`)")).unwrap_or_default();
    (error["code"].as_str().map(str::to_string), one_line(&format!("{message}{param}"), 300))
}

/// Pure — unit-tested. A refusal as the error the agent acts on. The words
/// matter: fallback.rs reads them (a "quota" is not retried, a 429 is).
pub fn refusal(status: Option<u16>, code: Option<&str>, message: &str) -> ModelError {
    let answered = match status {
        Some(status) => format!("the ChatGPT API answered {status}"),
        None => "the ChatGPT API stopped the answer".to_string(),
    };
    match code {
        // The plan's own limit or the cap set for rak: waiting a few seconds
        // does not help, so it is not retried.
        Some("subscription_sharing_usage_limit_exceeded") => ModelError::Fatal(format!(
            "{answered}: usage limit reached (your ChatGPT plan's quota, or the limit set for rak). Review it at {}",
            auth::USAGE_URL
        )),
        Some("subscription_sharing_user_not_eligible") => ModelError::Fatal(format!(
            "{answered}: this ChatGPT account, workspace or plan can't use its plan in other apps (403 not eligible): {message}"
        )),
        Some("subscription_sharing_invalid_user") => {
            ModelError::Fatal(format!("{answered}: the sign-in was not accepted (401): {message}. Run `rak login chatgpt` again"))
        }
        Some("subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable") => {
            ModelError::Retryable(format!("{answered}: {message}"))
        }
        _ => {
            let text = match code {
                Some(code) => format!("{answered}: {message} [{code}]"),
                None => format!("{answered}: {message}"),
            };
            let text = if status == Some(401) { format!("{text}. Run `rak login chatgpt` again") } else { text };
            match status {
                Some(408 | 429) => ModelError::Retryable(text),
                Some(s) if s >= 500 => ModelError::Retryable(text),
                // A failure part way with no status: the server's side, worth another try.
                None if matches!(code, Some("server_error" | "rate_limit_exceeded")) => ModelError::Retryable(text),
                _ => ModelError::Fatal(text),
            }
        }
    }
}

/// Pure — unit-tested. A chat-completions tool definition → this API's.
pub fn to_tool(definition: &Value) -> Value {
    let function = &definition["function"];
    json!({
        "type": "function",
        "name": function["name"],
        "description": function["description"],
        "parameters": function["parameters"],
        "strict": false,
    })
}

/// Pure — unit-tested. The agent's conversation (chat-completions shape) →
/// (instructions, input items).
///
/// - The first system message is the instructions; a later one is a
///   developer message (this route refuses the system role).
/// - An assistant message made here goes back as its own items, unchanged.
///   One from another provider goes as text and tool calls.
/// - A tool result is a `function_call_output` for its call.
pub fn to_input(messages: &[Value]) -> (Option<String>, Vec<Value>) {
    let mut instructions = None;
    let mut input = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        let text = message["content"].as_str().unwrap_or("");
        match message["role"].as_str().unwrap_or("") {
            "system" if i == 0 => instructions = Some(text.to_string()),
            "system" if !text.trim().is_empty() => input.push(json!({ "role": "developer", "content": text })),
            "user" => {
                let content = user_content(&message["content"]);
                if !content.is_null() {
                    input.push(json!({ "role": "user", "content": content }));
                }
            }
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": message["tool_call_id"],
                "output": if text.is_empty() { "(no output)" } else { text },
            })),
            "assistant" => match message.pointer(&format!("/native/{NATIVE}")).and_then(Value::as_array) {
                Some(items) => input.extend(items.iter().cloned()),
                None => {
                    if !text.trim().is_empty() {
                        input.push(json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text }] }));
                    }
                    for call in message["tool_calls"].as_array().into_iter().flatten() {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call["id"],
                            "name": call["function"]["name"],
                            "arguments": call["function"]["arguments"].as_str().unwrap_or("{}"),
                        }));
                    }
                }
            },
            _ => {}
        }
    }
    (instructions, input)
}

/// A user message's content: its text, or chat-completions content parts
/// (the images view_image brings) as this API's parts. Null when empty.
fn user_content(content: &Value) -> Value {
    let Some(parts) = content.as_array() else {
        return match content.as_str() {
            Some(text) if !text.trim().is_empty() => json!(text),
            _ => Value::Null,
        };
    };
    let parts: Vec<Value> = parts
        .iter()
        .filter_map(|part| match part["type"].as_str() {
            Some("text") => part["text"].as_str().filter(|t| !t.trim().is_empty()).map(|t| json!({ "type": "input_text", "text": t })),
            Some("image_url") => part["image_url"]["url"].as_str().map(|url| json!({ "type": "input_image", "image_url": url })),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        Value::Null
    } else {
        Value::Array(parts)
    }
}

/// A streamed reply being put together from its events.
#[derive(Default)]
pub struct Stream {
    /// The reply's output items, each as it is completed.
    items: Vec<Value>,
    usage: Usage,
    /// Answer text has been handed to the screen.
    pub shown_text: bool,
    /// `response.completed` arrived: only then is the reply whole.
    pub finished: bool,
    /// Why the reply was cut short (`response.incomplete`).
    incomplete: Option<String>,
}

impl Stream {
    /// Pure — unit-tested. Takes one line of the event stream and returns the
    /// answer text it carried, if any.
    pub fn push_line(&mut self, line: &str) -> Result<Option<String>, ModelError> {
        let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else { return Ok(None) };
        let Ok(event) = serde_json::from_str::<Value>(payload) else { return Ok(None) };
        match event["type"].as_str().unwrap_or("") {
            "response.output_text.delta" => {
                let text = event["delta"].as_str().unwrap_or("");
                if text.is_empty() {
                    return Ok(None);
                }
                self.shown_text = true;
                Ok(Some(text.to_string()))
            }
            "response.output_item.done" => {
                if event["item"].is_object() {
                    self.items.push(event["item"].clone());
                }
                Ok(None)
            }
            "response.completed" | "response.incomplete" => {
                let response = &event["response"];
                self.usage = usage_of(&response["usage"]);
                // The items normally came one by one; the whole list is the
                // fallback for a server that sends only this.
                if self.items.is_empty() {
                    self.items = response["output"].as_array().cloned().unwrap_or_default();
                }
                if event["type"] == "response.incomplete" {
                    self.incomplete = Some(
                        response.pointer("/incomplete_details/reason").and_then(Value::as_str).unwrap_or("no reason given").to_string(),
                    );
                }
                self.finished = true;
                Ok(None)
            }
            "response.failed" => {
                let error = &event["response"]["error"];
                Err(refusal(None, error["code"].as_str(), error["message"].as_str().unwrap_or("the response failed")))
            }
            "error" => {
                let error = if event["error"].is_object() { &event["error"] } else { &event };
                Err(refusal(None, error["code"].as_str(), error["message"].as_str().unwrap_or("an error with no message")))
            }
            _ => Ok(None),
        }
    }

    /// Pure — unit-tested. The reply, once the stream has ended.
    pub fn finish(self) -> Result<Reply, ModelError> {
        if !self.finished {
            // The stream ended with no closing event: not a whole reply.
            return Err(if self.shown_text {
                ModelError::Fatal("the answer stopped arriving part way: the stream ended before it was complete".to_string())
            } else {
                ModelError::Retryable("the answer stopped arriving: the stream ended before it was complete".to_string())
            });
        }
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for item in &self.items {
            match item["type"].as_str() {
                Some("message") => {
                    for part in item["content"].as_array().into_iter().flatten() {
                        match part["type"].as_str() {
                            Some("output_text") => content.push_str(part["text"].as_str().unwrap_or("")),
                            Some("refusal") => content.push_str(part["refusal"].as_str().unwrap_or("")),
                            _ => {}
                        }
                    }
                }
                Some("function_call") => tool_calls.push(ToolCall {
                    id: item["call_id"].as_str().unwrap_or("").to_string(),
                    name: item["name"].as_str().unwrap_or("").to_string(),
                    arguments: item["arguments"].as_str().map(str::to_string).unwrap_or_else(|| "{}".to_string()),
                }),
                _ => {}
            }
        }
        if let Some(reason) = &self.incomplete {
            if content.is_empty() && tool_calls.is_empty() {
                return Err(ModelError::Fatal(format!("the ChatGPT API cut the answer short: {reason}")));
            }
            // A tool call cut short has arguments that end mid-way: not run.
            tool_calls.clear();
        }
        let native: Vec<Value> = self.items.into_iter().filter_map(for_sending_back).collect();
        Ok(Reply { content, tool_calls, usage: self.usage, native: (!native.is_empty()).then(|| json!({ NATIVE: native })) })
    }
}

/// An output item as it goes back in the next request. Nothing is stored on
/// the server (`store: false`), so the item's server id means nothing there
/// and is left out; reasoning without its encrypted form can't go back at all.
fn for_sending_back(mut item: Value) -> Option<Value> {
    let kind = item["type"].as_str()?.to_string();
    if !matches!(kind.as_str(), "message" | "function_call" | "reasoning") {
        return None;
    }
    if kind == "reasoning" && item["encrypted_content"].as_str().is_none_or(str::is_empty) {
        return None;
    }
    let fields = item.as_object_mut()?;
    fields.remove("id");
    fields.remove("status");
    Some(item)
}

/// Pure — unit-tested.
fn usage_of(usage: &Value) -> Usage {
    Usage {
        input: usage["input_tokens"].as_u64().unwrap_or(0),
        output: usage["output_tokens"].as_u64().unwrap_or(0),
        cached: usage.pointer("/input_tokens_details/cached_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_write: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ChatGpt {
        let mut m = ChatGpt::new("https://api.openai.com/v1/", "gpt-test");
        m.fixed_token = Some("token".into());
        m
    }

    #[test]
    fn a_request_follows_the_plan_route_s_rules() {
        let mut m = model();
        assert_eq!(m.base_url, "https://api.openai.com/v1");
        let messages = [json!({ "role": "system", "content": "Be brief." }), json!({ "role": "user", "content": "hi" })];
        let body = m.request_body(&messages, &[]);
        assert_eq!((&body["store"], &body["stream"]), (&json!(false), &json!(true)), "both are required on this route");
        assert_eq!(body["instructions"], "Be brief.");
        assert_eq!(body["input"], json!([{ "role": "user", "content": "hi" }]), "no system message in the input");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        // The same cache key on every request of this conversation; another model object has its own.
        assert_eq!(body["prompt_cache_key"], m.request_body(&messages, &[])["prompt_cache_key"]);
        assert!(body["prompt_cache_key"].as_str().is_some_and(|key| key.len() >= 32));
        assert_ne!(body["prompt_cache_key"], model().request_body(&messages, &[])["prompt_cache_key"]);
        for refused in
            ["max_output_tokens", "temperature", "top_p", "truncation", "metadata", "user", "previous_response_id", "tools", "reasoning"]
        {
            assert!(body.get(refused).is_none(), "{refused} must not be sent");
        }
        m.effort = Some("max".into());
        let tool =
            json!({ "type": "function", "function": { "name": "read_file", "description": "Reads.", "parameters": { "type": "object" } } });
        let body = m.request_body(&messages, &[tool]);
        assert_eq!(body["reasoning"], json!({ "effort": "max" }), "the plan's models take low to max as they are");
        assert_eq!(
            body["tools"],
            json!([{ "type": "function", "name": "read_file", "description": "Reads.", "parameters": { "type": "object" }, "strict": false }])
        );
    }

    #[test]
    fn the_conversation_becomes_input_items() {
        let native = json!({ "chatgpt": [
            { "type": "reasoning", "encrypted_content": "enc", "summary": [] },
            { "type": "function_call", "call_id": "c1", "name": "read_file", "arguments": "{\"path\":\"a\"}" }
        ]});
        let messages = [
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "look" }),
            // Made here: sent back as its own items, reasoning included.
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1", "function": { "name": "read_file", "arguments": "{\"path\":\"a\"}" } }], "native": native }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "" }),
            // From another provider (Anthropic's native is a list): text and calls.
            json!({ "role": "assistant", "content": "Now b.", "tool_calls": [{ "id": "toolu_2", "function": { "name": "read_file", "arguments": "{\"path\":\"b\"}" } }], "native": [{ "type": "thinking" }] }),
            json!({ "role": "tool", "tool_call_id": "toolu_2", "content": "bbb" }),
            json!({ "role": "system", "content": "A note from rak." }),
            json!({ "role": "user", "content": [{ "type": "text", "text": "and this" }, { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }] }),
            json!({ "role": "user", "content": "  " }),
            json!({ "role": "assistant", "content": "" }),
        ];
        let (instructions, input) = to_input(&messages);
        assert_eq!(instructions.as_deref(), Some("sys"));
        assert_eq!(
            Value::Array(input),
            json!([
                { "role": "user", "content": "look" },
                { "type": "reasoning", "encrypted_content": "enc", "summary": [] },
                { "type": "function_call", "call_id": "c1", "name": "read_file", "arguments": "{\"path\":\"a\"}" },
                { "type": "function_call_output", "call_id": "c1", "output": "(no output)" },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Now b." }] },
                { "type": "function_call", "call_id": "toolu_2", "name": "read_file", "arguments": "{\"path\":\"b\"}" },
                { "type": "function_call_output", "call_id": "toolu_2", "output": "bbb" },
                { "role": "developer", "content": "A note from rak." },
                { "role": "user", "content": [{ "type": "input_text", "text": "and this" }, { "type": "input_image", "image_url": "data:image/png;base64,AAAA" }] },
            ])
        );
    }

    fn feed(events: &[Value]) -> (Stream, String) {
        let mut stream = Stream::default();
        let mut shown = String::new();
        for event in events {
            stream.push_line(&format!("event: {}\n", event["type"].as_str().unwrap())).unwrap();
            if let Some(text) = stream.push_line(&format!("data: {event}\n")).unwrap() {
                shown.push_str(&text);
            }
            stream.push_line("\n").unwrap();
        }
        (stream, shown)
    }

    #[test]
    fn a_streamed_reply_gives_text_tool_calls_usage_and_items_to_send_back() {
        let (stream, shown) = feed(&[
            json!({ "type": "response.created", "response": { "id": "resp_1" } }),
            json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "id": "rs_1", "encrypted_content": "enc", "summary": [] } }),
            json!({ "type": "response.output_text.delta", "delta": "Look" }),
            json!({ "type": "response.output_text.delta", "delta": "ing." }),
            json!({ "type": "response.output_item.done", "item": { "type": "message", "id": "msg_1", "status": "completed", "role": "assistant", "content": [{ "type": "output_text", "text": "Looking." }] } }),
            json!({ "type": "response.function_call_arguments.delta", "delta": "{\"pa" }),
            json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1", "status": "completed", "call_id": "call_1", "name": "read_file", "arguments": "{\"path\":\"x\"}" } }),
            json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 120, "output_tokens": 9, "input_tokens_details": { "cached_tokens": 100 } } } }),
        ]);
        assert_eq!(shown, "Looking.");
        assert!(stream.finished && stream.shown_text);
        let reply = stream.finish().unwrap();
        assert_eq!(reply.content, "Looking.");
        assert_eq!(reply.tool_calls, [ToolCall { id: "call_1".into(), name: "read_file".into(), arguments: "{\"path\":\"x\"}".into() }]);
        assert_eq!(reply.usage, Usage { input: 120, output: 9, cached: 100, cache_write: 0 });
        // The items go back without the server's ids: nothing is stored there.
        assert_eq!(
            reply.native,
            Some(json!({ "chatgpt": [
                { "type": "reasoning", "encrypted_content": "enc", "summary": [] },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Looking." }] },
                { "type": "function_call", "call_id": "call_1", "name": "read_file", "arguments": "{\"path\":\"x\"}" },
            ]}))
        );
        // And the next request carries them as they are.
        let (_, input) = to_input(&[json!({ "role": "assistant", "content": reply.content, "native": reply.native })]);
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "reasoning");
    }

    #[test]
    fn a_reply_is_whole_only_after_its_closing_event() {
        // Ended with nothing shown: worth another try.
        let (stream, _) = feed(&[json!({ "type": "response.created" })]);
        assert!(matches!(stream.finish(), Err(ModelError::Retryable(m)) if m.contains("stopped arriving")));
        // Ended after text was shown: a retry would show it twice.
        let (stream, _) = feed(&[json!({ "type": "response.output_text.delta", "delta": "Half" })]);
        assert!(matches!(stream.finish(), Err(ModelError::Fatal(m)) if m.contains("part way")));
        // Only the closing event carried the output.
        let (stream, _) = feed(&[json!({ "type": "response.completed", "response": {
            "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "All at once." }] }],
            "usage": { "input_tokens": 3, "output_tokens": 4 } } })]);
        assert_eq!(stream.finish().unwrap().content, "All at once.");
        // Cut short: the text is kept, a half-written tool call is not run.
        let (stream, _) = feed(&[
            json!({ "type": "response.output_item.done", "item": { "type": "message", "content": [{ "type": "output_text", "text": "Part" }] } }),
            json!({ "type": "response.output_item.done", "item": { "type": "function_call", "call_id": "c", "name": "run", "arguments": "{\"cmd" } }),
            json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "max_output_tokens" } } }),
        ]);
        let reply = stream.finish().unwrap();
        assert_eq!((reply.content.as_str(), reply.tool_calls.len()), ("Part", 0));
        let (stream, _) =
            feed(&[json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "content_filter" } } })]);
        assert!(matches!(stream.finish(), Err(ModelError::Fatal(m)) if m.contains("content_filter")));
    }

    #[test]
    fn refusals_say_what_to_do_and_are_classified_for_falling_back() {
        use crate::fallback::{classify, Failure};
        let limit = refusal(Some(429), Some("subscription_sharing_usage_limit_exceeded"), "limit");
        assert!(matches!(&limit, ModelError::Fatal(m) if m.contains(auth::USAGE_URL)));
        assert_eq!(classify(&limit), Failure::Credit, "a plan limit is not retried; another model may be used");
        // The same limit, arriving after the stream opened.
        let mut stream = Stream::default();
        let failed = json!({ "type": "response.failed", "response": { "error": { "code": "subscription_sharing_usage_limit_exceeded", "message": "x" } } });
        let error = stream.push_line(&format!("data: {failed}")).unwrap_err();
        assert_eq!(classify(&error), Failure::Credit);

        assert_eq!(classify(&refusal(Some(503), Some("subscription_sharing_usage_unavailable"), "try later")), Failure::Transient);
        assert_eq!(classify(&refusal(Some(429), None, "slow down")), Failure::RateLimit);
        assert_eq!(classify(&refusal(Some(500), None, "oops")), Failure::Transient);
        assert_eq!(classify(&refusal(None, Some("server_error"), "oops")), Failure::Transient);
        let signed_out = refusal(Some(401), None, "token not accepted");
        assert!(matches!(&signed_out, ModelError::Fatal(m) if m.contains("rak login chatgpt")));
        assert_eq!(classify(&signed_out), Failure::Config);
        let not_eligible = refusal(Some(403), Some("subscription_sharing_user_not_eligible"), "workspace policy");
        assert_eq!(classify(&not_eligible), Failure::Config);
        let unsupported = refusal(Some(400), Some("subscription_sharing_unsupported_capability"), "not supported (in `tools`)");
        assert!(
            matches!(&unsupported, ModelError::Fatal(m) if m.contains("[subscription_sharing_unsupported_capability]") && m.contains("`tools`"))
        );

        assert_eq!(
            error_parts(&json!({ "error": { "code": "c", "message": "m", "param": "tools" } }), ""),
            (Some("c".into()), "m (in `tools`)".into())
        );
        assert_eq!(error_parts(&json!({ "detail": "Region not served" }), ""), (None, "Region not served".into()));
        assert_eq!(error_parts(&Value::Null, "Bad Gateway"), (None, "Bad Gateway".into()));
    }

    #[test]
    fn the_model_list_keeps_the_server_s_order_and_only_what_is_for_listing() {
        let body = json!({ "models": [
            { "slug": "gpt-b", "display_name": "B", "visibility": "list", "context_window": 400000, "input_modalities": ["text", "image"] },
            { "slug": "gpt-hidden", "visibility": "hide" },
            { "slug": "gpt-a", "visibility": "list" }
        ]});
        assert_eq!(model_names(&body), ["gpt-b", "gpt-a"]);
        assert_eq!(context_of(&body, "gpt-b"), Some(400_000));
        assert_eq!(context_of(&body, "gpt-a"), None);
        assert_eq!((sees_of(&body, "gpt-b"), sees_of(&body, "gpt-a"), sees_of(&body, "gpt-none")), (Some(true), None, None));
        assert_eq!(model_names(&json!({ "data": [{ "id": "gpt-x" }] })), ["gpt-x"]);
        assert!(model_names(&json!({})).is_empty());
    }

    /// A stand-in for the API: answers each request with the next of
    /// `answers` (status, body) and returns what it was sent (headers, body).
    fn server(answers: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, answer) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 8192];
                let (head, body) = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                            .unwrap_or(0);
                        if body.len() >= length {
                            break (head.to_string(), body.to_string());
                        }
                    }
                    if n == 0 {
                        break (text, String::new());
                    }
                };
                seen.push((head, serde_json::from_str(&body).unwrap_or(Value::Null)));
                let kind = if status == 200 { "text/event-stream" } else { "application/json" };
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                )
                .unwrap();
            }
            seen
        });
        (url, handle)
    }

    fn sse(events: &[Value]) -> String {
        events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect()
    }

    #[test]
    fn a_request_goes_out_with_the_token_and_a_default_effort_is_dropped_when_refused() {
        let done = sse(&[
            json!({ "type": "response.output_text.delta", "delta": "ok" }),
            json!({ "type": "response.output_item.done", "item": { "type": "message", "content": [{ "type": "output_text", "text": "ok" }] } }),
            json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 5, "output_tokens": 1 } } }),
        ]);
        let refused =
            json!({ "error": { "code": "unsupported_value", "message": "effort not supported", "param": "reasoning.effort" } }).to_string();
        let (url, handle) = server(vec![(400, refused), (200, done)]);
        let mut m = ChatGpt::new(&url, "gpt-unique-effort-test");
        m.fixed_token = Some("tok-1".into());
        m.effort = Some("medium".into());
        m.effort_optional = true;
        let mut shown = String::new();
        let reply = m.chat_stream(&[json!({ "role": "user", "content": "hi" })], &[], &mut |t| shown.push_str(t)).unwrap();
        assert_eq!((reply.content.as_str(), shown.as_str()), ("ok", "ok"));
        assert_eq!(m.effort, None, "the model takes no effort: none from then on");
        let seen = handle.join().unwrap();
        assert!(seen[0].0.starts_with("POST /v1/responses ") && seen[0].0.to_lowercase().contains("authorization: bearer tok-1"));
        assert_eq!(seen[0].1["reasoning"], json!({ "effort": "medium" }));
        // Both requests carry the conversation's session id, for the prompt cache.
        let session = |head: &str| head.lines().find_map(|l| l.to_lowercase().strip_prefix("session_id:").map(|v| v.trim().to_string()));
        assert!(session(&seen[0].0).is_some_and(|id| id.len() >= 32));
        assert_eq!(session(&seen[0].0), session(&seen[1].0));
        assert!(seen[1].1.get("reasoning").is_none());
    }

    #[test]
    fn a_plan_limit_stops_the_request_with_where_to_look() {
        let body =
            json!({ "error": { "code": "subscription_sharing_usage_limit_exceeded", "message": "Usage limit reached" } }).to_string();
        let (url, handle) = server(vec![(429, body)]);
        let mut m = ChatGpt::new(&url, "gpt-test");
        m.fixed_token = Some("tok".into());
        let error = m.chat(&[json!({ "role": "user", "content": "hi" })], &[]).unwrap_err();
        assert!(matches!(error, ModelError::Fatal(m) if m.contains("answered 429") && m.contains("chatgpt.com/settings/usage")));
        handle.join().unwrap();
    }

    #[test]
    fn with_no_sign_in_the_request_says_how_to_sign_in() {
        let dir = crate::tools::testing::temp_dir("chatgpt-none");
        let mut m = ChatGpt::new("http://127.0.0.1:1/v1", "gpt-test");
        m.account_file = Some(dir.join("chatgpt.json"));
        let error = m.chat(&[json!({ "role": "user", "content": "hi" })], &[]).unwrap_err();
        assert!(matches!(error, ModelError::Fatal(m) if m == auth::NOT_SIGNED_IN));
        assert_eq!(m.list_models().unwrap_err(), auth::NOT_SIGNED_IN);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
