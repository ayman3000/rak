//! The model side: one trait the agent loop talks to, and a client for any
//! server that speaks OpenAI's chat-completions API (OpenAI, OpenRouter,
//! Ollama, Hugging Face's router, vLLM, llama.cpp, LM Studio…).

use crate::interrupt::Stop;
use crate::text::one_line;
use serde_json::{json, Value};
use std::io::BufRead;
use std::time::Duration;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Usage {
    /// The whole prompt, cached part included.
    pub input: u64,
    pub output: u64,
    /// The part of `input` read from the server's prompt cache.
    pub cached: u64,
    /// The part of `input` written to the cache this time (Anthropic reports it).
    pub cache_write: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cached += other.cached;
        self.cache_write += other.cache_write;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments as the model sent them: a JSON object in a string.
    pub arguments: String,
}

#[derive(Debug, Default, Clone)]
pub struct Reply {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// The reply in the provider's own form, when sending it back as text and
    /// tool calls would lose something (Anthropic's thinking blocks). The
    /// agent keeps it with the message; only that provider reads it.
    pub native: Option<Value>,
}

#[derive(Debug)]
pub enum ModelError {
    /// Worth trying again: rate limit, server error, dropped connection.
    Retryable(String),
    Fatal(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Retryable(m) | ModelError::Fatal(m) => f.write_str(m),
        }
    }
}

/// What the agent loop needs from a model. `messages` and `tools` are in
/// OpenAI chat-completions shape.
pub trait Model {
    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError>;

    /// Like `chat`, handing over the answer's text as it arrives. A model that
    /// cannot stream hands it over in one piece.
    fn chat_stream(&mut self, messages: &[Value], tools: &[Value], on_text: &mut dyn FnMut(&str)) -> Result<Reply, ModelError> {
        let reply = self.chat(messages, tools)?;
        if !reply.content.is_empty() {
            on_text(&reply.content);
        }
        Ok(reply)
    }

    /// The ids of the models the server offers, sorted.
    fn list_models(&self) -> Result<Vec<String>, String> {
        Err("this server can't list its models".to_string())
    }

    /// The model's context window in tokens, when the server says.
    fn context_window(&self) -> Option<usize> {
        None
    }

    /// Whether the model reads images: Some when the server says, None when it
    /// can't tell (then view_image is offered and the server decides).
    fn sees_images(&self) -> Option<bool> {
        None
    }
}

/// What the person sees when they stop a request mid-answer.
pub const STOPPED: &str = "stopped";

pub struct OpenAICompatible {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// Sent as `reasoning_effort` when set (low / medium / high).
    pub effort: Option<String>,
    /// Sent as `max_tokens` when set; otherwise the server's default applies.
    pub max_tokens: Option<u64>,
    /// Checked between pieces of a streamed answer.
    pub stop: Stop,
    /// How long a streamed answer may send nothing (see stall.rs).
    pub stall_base: Duration,
    /// For short requests (model lists, Ollama's model details).
    http: ureq::Agent,
    clients: crate::stall::Clients,
}

impl OpenAICompatible {
    pub fn new(base_url: &str, api_key: Option<String>, model: &str) -> Self {
        let http = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(30)).timeout_read(Duration::from_secs(60)).build();
        OpenAICompatible {
            stall_base: crate::stall::base_for(base_url, false),
            clients: Default::default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model: model.to_string(),
            effort: None,
            max_tokens: None,
            stop: Stop::new(),
            http,
        }
    }

    /// Pure — unit-tested.
    pub fn request_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        // A message's `native` form is for the provider that made it, not for
        // this one; only the reasoning a chat-completions server sent comes
        // back, under its own field (see `reasoning_native`).
        let messages: Vec<Value> = messages
            .iter()
            .map(|m| match m.get("native") {
                Some(native) => {
                    let mut plain = m.clone();
                    plain.as_object_mut().map(|o| o.remove("native"));
                    if let Some(text) = native.get(REASONING_FIELD).and_then(Value::as_str) {
                        plain[REASONING_FIELD] = json!(text);
                    }
                    plain
                }
                None => m.clone(),
            })
            .collect();
        let mut body = json!({ "model": self.model, "messages": messages });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(effort) = &self.effort {
            body["reasoning_effort"] = json!(effort);
        }
        if let Some(max) = self.max_tokens {
            body["max_tokens"] = json!(max);
        }
        body
    }
}

impl OpenAICompatible {
    /// `read`: how long the server may send nothing.
    fn post(&self, body: Value, read: Duration) -> Result<ureq::Response, ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self.clients.with_read_limit(read).post(&url);
        if let Some(key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        match request.send_json(body) {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                let message = format!("the model server answered {code}: {}", one_line(&error_message(&text), 300));
                if code == 408 || code == 429 || code >= 500 {
                    Err(ModelError::Retryable(message))
                } else {
                    Err(ModelError::Fatal(message))
                }
            }
            Err(ureq::Error::Transport(t)) => Err(ModelError::Retryable(format!("could not reach {url}: {t}"))),
        }
    }

    /// The model's context window in tokens, when the server is an Ollama
    /// one and says (`POST /api/show`). None for every other server.
    fn ollama_window(&self) -> Option<usize> {
        context_length(&self.ollama_show()?)
    }

    /// Ollama's description of the model (`POST /api/show`); None for other servers.
    fn ollama_show(&self) -> Option<Value> {
        let root = self.base_url.strip_suffix("/v1")?;
        let mut request = self.http.post(&format!("{root}/api/show")).timeout(Duration::from_secs(4));
        // Ollama's cloud (ollama.com) answers only with the key.
        if let Some(key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        request.send_json(json!({ "model": self.model })).ok()?.into_json().ok()
    }

    /// The ids of the models the server offers (`GET /models`), sorted.
    fn models(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/models", self.base_url);
        let mut request = self.http.get(&url).timeout(Duration::from_secs(20));
        if let Some(key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        let body: Value = match request.call() {
            Ok(response) => response.into_json().map_err(|e| format!("unreadable reply from {url}: {e}"))?,
            Err(ureq::Error::Status(code, _)) => return Err(format!("{url} answered {code}")),
            Err(ureq::Error::Transport(t)) => return Err(format!("could not reach {url}: {t}")),
        };
        let mut ids: Vec<String> =
            body["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
        ids.sort();
        Ok(ids)
    }
}

impl Model for OpenAICompatible {
    fn list_models(&self) -> Result<Vec<String>, String> {
        self.models()
    }

    fn context_window(&self) -> Option<usize> {
        self.ollama_window()
    }

    /// Ollama lists a model's capabilities; "vision" means it reads images.
    fn sees_images(&self) -> Option<bool> {
        let show = self.ollama_show()?;
        let capabilities = show["capabilities"].as_array()?;
        Some(capabilities.iter().any(|c| c == "vision"))
    }

    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError> {
        let body = self.request_body(messages, tools);
        let read = crate::stall::limit(crate::stall::WHOLE_ANSWER, body.to_string().len());
        let body: Value = self
            .post(body, read)?
            .into_json()
            .map_err(|e| ModelError::Retryable(format!("unreadable reply from the model server: {e}")))?;
        parse_reply(&body)
    }

    fn chat_stream(&mut self, messages: &[Value], tools: &[Value], on_text: &mut dyn FnMut(&str)) -> Result<Reply, ModelError> {
        let mut body = self.request_body(messages, tools);
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
        let read = crate::stall::limit(self.stall_base, body.to_string().len());
        let mut lines = std::io::BufReader::new(self.post(body, read)?.into_reader());
        let mut streamed = StreamedReply::default();
        let mut line = String::new();
        loop {
            if self.stop.requested() {
                return Err(ModelError::Fatal(STOPPED.to_string()));
            }
            line.clear();
            match lines.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Some(text) = streamed.push_line(&line)? {
                        on_text(&text);
                    }
                }
                // Text already on screen cannot be taken back, so a second try
                // would show the answer twice.
                Err(e) if streamed.content.is_empty() => {
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
        Ok(streamed.finish())
    }
}

/// Pure — unit-tested. The window in Ollama's description of a model: the
/// `<family>.context_length` entry of `model_info`.
pub fn context_length(show: &Value) -> Option<usize> {
    show["model_info"]
        .as_object()?
        .iter()
        .find(|(key, _)| key.ends_with(".context_length"))
        .and_then(|(_, value)| value.as_u64())
        .filter(|n| *n > 0)
        .map(|n| n as usize)
}

/// The field DeepSeek, vLLM and many providers put a reasoning model's
/// thinking in. Within one request (the tool calls of one task) it goes back
/// with each assistant message, as DeepSeek asks; a new request drops it
/// (`Agent::run`). Only this name is echoed: it is sent back only to a server
/// that sent it, and other names (`reasoning`) are not accepted everywhere.
pub const REASONING_FIELD: &str = "reasoning_content";

/// A reply's reasoning, kept as its native form so it can go back.
fn reasoning_native(text: &str) -> Option<Value> {
    (!text.is_empty()).then(|| json!({ REASONING_FIELD: text }))
}

/// A streamed reply being put together from its events.
#[derive(Default)]
pub struct StreamedReply {
    pub content: String,
    reasoning: String,
    /// (id, name, arguments so far)
    calls: Vec<(String, String, String)>,
    usage: Usage,
}

impl StreamedReply {
    /// Pure — unit-tested. Takes one line of the event stream and returns the
    /// answer text it carried, if any.
    pub fn push_line(&mut self, line: &str) -> Result<Option<String>, ModelError> {
        let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else { return Ok(None) };
        let Ok(event) = serde_json::from_str::<Value>(payload) else { return Ok(None) }; // "[DONE]", keep-alives
        if let Some(err) = event.get("error").filter(|e| !e.is_null()) {
            let message = err.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| err.to_string());
            return Err(ModelError::Fatal(format!("the model server reported an error: {message}")));
        }
        if let Some(usage) = event.get("usage").filter(|u| u.is_object()) {
            self.usage = openai_usage(usage);
        }
        let Some(delta) = event.pointer("/choices/0/delta") else { return Ok(None) };
        for (position, call) in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let id = call["id"].as_str().unwrap_or("");
            let name = call["function"]["name"].as_str().unwrap_or("");
            let arguments = match &call["function"]["arguments"] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            let mut index = call["index"].as_u64().map(|i| i as usize).unwrap_or(position);
            // Some servers give every call index 0. A new id on a call that
            // already has a name is a new call.
            if let Some(existing) = self.calls.get(index) {
                if !id.is_empty() && !existing.0.is_empty() && existing.0 != id && !existing.1.is_empty() {
                    index = self.calls.len();
                }
            }
            while self.calls.len() <= index {
                self.calls.push(Default::default());
            }
            let slot = &mut self.calls[index];
            if !id.is_empty() {
                slot.0 = id.to_string();
            }
            slot.1.push_str(name);
            slot.2.push_str(&arguments);
        }
        if let Some(text) = delta.get(REASONING_FIELD).and_then(Value::as_str) {
            self.reasoning.push_str(text);
        }
        match delta.get("content").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => {
                self.content.push_str(text);
                Ok(Some(text.to_string()))
            }
            _ => Ok(None),
        }
    }

    pub fn finish(self) -> Reply {
        let tool_calls = self
            .calls
            .into_iter()
            .enumerate()
            .filter(|(_, (_, name, _))| !name.is_empty())
            .map(|(i, (id, name, arguments))| ToolCall {
                id: if id.is_empty() { format!("call_{i}") } else { id },
                name,
                arguments: if arguments.trim().is_empty() { "{}".to_string() } else { arguments },
            })
            .collect();
        Reply { content: self.content, tool_calls, usage: self.usage, native: reasoning_native(&self.reasoning) }
    }
}

/// Pure — unit-tested. The message in an error body (`{"error": {"message": …}}`,
/// `{"error": "…"}` or `{"message": …}`), or the body itself.
pub fn error_message(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else { return body.to_string() };
    ["/error/message", "/error", "/message"]
        .iter()
        .find_map(|path| value.pointer(path).and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| body.to_string())
}

/// Pure — unit-tested. Reads one chat-completions response.
pub fn parse_reply(body: &Value) -> Result<Reply, ModelError> {
    if let Some(err) = body.get("error") {
        let message = err.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| err.to_string());
        return Err(ModelError::Fatal(format!("the model server reported an error: {message}")));
    }
    let message = body
        .pointer("/choices/0/message")
        .ok_or_else(|| ModelError::Retryable(format!("reply had no message: {}", one_line(&body.to_string(), 200))))?;

    let content = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
    let mut tool_calls = Vec::new();
    for (i, call) in message.get("tool_calls").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let function = &call["function"];
        let name = function["name"].as_str().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        // Most servers send the arguments as a string; a few send the object.
        let arguments = match &function["arguments"] {
            Value::String(s) => s.clone(),
            Value::Null => "{}".to_string(),
            other => other.to_string(),
        };
        let id = call["id"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("call_{i}"));
        tool_calls.push(ToolCall { id, name, arguments });
    }
    let usage = openai_usage(&body["usage"]);
    let reasoning = message.get(REASONING_FIELD).and_then(Value::as_str).unwrap_or("");
    Ok(Reply { content, tool_calls, usage, native: reasoning_native(reasoning) })
}

/// Pure — unit-tested. A chat-completions `usage` object. Servers that cache
/// say how much of the prompt came from it in `prompt_tokens_details`.
pub fn openai_usage(usage: &Value) -> Usage {
    Usage {
        input: usage["prompt_tokens"].as_u64().unwrap_or(0),
        output: usage["completion_tokens"].as_u64().unwrap_or(0),
        cached: usage.pointer("/prompt_tokens_details/cached_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_write: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_tool_calls_and_usage() {
        let body = json!({
            "choices": [{ "message": {
                "content": "Looking.",
                "tool_calls": [
                    { "id": "a1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"x\"}" } },
                    { "type": "function", "function": { "name": "list_dir", "arguments": { "path": "." } } }
                ]
            }}],
            "usage": { "prompt_tokens": 120, "completion_tokens": 9 }
        });
        let reply = parse_reply(&body).unwrap();
        assert_eq!(reply.content, "Looking.");
        assert_eq!(reply.tool_calls[0], ToolCall { id: "a1".into(), name: "read_file".into(), arguments: "{\"path\":\"x\"}".into() });
        assert_eq!(reply.tool_calls[1].id, "call_1");
        assert_eq!(reply.tool_calls[1].arguments, "{\"path\":\".\"}");
        assert_eq!(reply.usage, Usage { input: 120, output: 9, ..Default::default() });
    }

    #[test]
    fn null_content_is_empty_text() {
        let reply = parse_reply(&json!({ "choices": [{ "message": { "content": null } }] })).unwrap();
        assert_eq!(reply.content, "");
        assert!(reply.tool_calls.is_empty());
    }

    #[test]
    fn an_error_body_is_reduced_to_its_message() {
        assert_eq!(error_message(r#"{"error": {"message": "model not found", "type": "x"}}"#), "model not found");
        assert_eq!(error_message(r#"{"error": "unauthorized"}"#), "unauthorized");
        assert_eq!(error_message("Bad Gateway"), "Bad Gateway");
    }

    #[test]
    fn a_server_error_body_is_reported() {
        let err = parse_reply(&json!({ "error": { "message": "model not found" } })).unwrap_err();
        assert!(matches!(err, ModelError::Fatal(m) if m.contains("model not found")));
    }

    #[test]
    fn effort_and_tools_are_sent_only_when_set() {
        let mut p = OpenAICompatible::new("http://localhost:1/v1/", None, "m");
        assert_eq!(p.base_url, "http://localhost:1/v1");
        let plain = p.request_body(&[json!({"role": "user", "content": "hi"})], &[]);
        assert!(plain.get("tools").is_none() && plain.get("reasoning_effort").is_none());
        p.effort = Some("low".into());
        let body = p.request_body(&[], &[json!({"type": "function"})]);
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_streamed_reply_is_put_together_from_its_pieces() {
        let mut r = StreamedReply::default();
        assert_eq!(r.push_line(": keep-alive").unwrap(), None);
        assert_eq!(r.push_line(r#"data: {"choices":[{"delta":{"role":"assistant","content":"Look"}}]}"#).unwrap().as_deref(), Some("Look"));
        assert_eq!(r.push_line(r#"data: {"choices":[{"delta":{"content":"ing."}}]}"#).unwrap().as_deref(), Some("ing."));
        r.push_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a1","function":{"name":"read_file","arguments":"{\"pa"}}]}}]}"#,
        )
        .unwrap();
        r.push_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"x\"}"}}]}}]}"#).unwrap();
        r.push_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"a2","function":{"name":"list_dir","arguments":""}}]}}]}"#,
        )
        .unwrap();
        r.push_line(r#"data: {"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":9}}"#).unwrap();
        assert_eq!(r.push_line("data: [DONE]").unwrap(), None);
        let reply = r.finish();
        assert_eq!(reply.content, "Looking.");
        assert_eq!(reply.tool_calls[0], ToolCall { id: "a1".into(), name: "read_file".into(), arguments: "{\"path\":\"x\"}".into() });
        assert_eq!(reply.tool_calls[1], ToolCall { id: "a2".into(), name: "list_dir".into(), arguments: "{}".into() });
        assert_eq!(reply.usage, Usage { input: 120, output: 9, ..Default::default() });
    }

    #[test]
    fn streamed_calls_that_all_claim_index_zero_stay_apart() {
        let mut r = StreamedReply::default();
        r.push_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read_file","arguments":"{\"path\":\"x\"}"}}]}}]}"#).unwrap();
        r.push_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"b","function":{"name":"read_file","arguments":"{\"path\":\"y\"}"}}]}}]}"#).unwrap();
        let reply = r.finish();
        assert_eq!(reply.tool_calls.len(), 2);
        assert_eq!(reply.tool_calls[1].arguments, "{\"path\":\"y\"}");
    }

    #[test]
    fn the_window_is_read_from_ollamas_model_description() {
        let show = json!({ "model_info": { "general.architecture": "kimi", "kimi.context_length": 262144, "kimi.block_count": 40 } });
        assert_eq!(context_length(&show), Some(262_144));
        assert_eq!(context_length(&json!({ "model_info": {} })), None);
        assert_eq!(context_length(&json!({ "error": "not found" })), None);
    }

    #[test]
    fn reasoning_content_is_kept_and_sent_back_under_its_own_name() {
        let mut r = StreamedReply::default();
        r.push_line(r#"data: {"choices":[{"delta":{"reasoning_content":"Think "}}]}"#).unwrap();
        r.push_line(r#"data: {"choices":[{"delta":{"reasoning_content":"more."}}]}"#).unwrap();
        r.push_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"list_dir","arguments":"{}"}}]}}]}"#,
        )
        .unwrap();
        let reply = r.finish();
        assert_eq!(reply.native, Some(json!({ "reasoning_content": "Think more." })));
        let p = OpenAICompatible::new("http://x/v1", None, "m");
        let message = json!({ "role": "assistant", "content": "", "native": reply.native });
        let body = p.request_body(&[message], &[]);
        assert_eq!(body["messages"][0]["reasoning_content"], "Think more.");
        assert!(body["messages"][0].get("native").is_none());
        let whole = parse_reply(&json!({ "choices": [{ "message": { "content": "hi", "reasoning_content": "hm" } }] })).unwrap();
        assert_eq!(whole.native, Some(json!({ "reasoning_content": "hm" })));
        // A server that sends no reasoning gets none back.
        assert!(parse_reply(&json!({ "choices": [{ "message": { "content": "hi" } }] })).unwrap().native.is_none());
    }

    #[test]
    fn cached_tokens_and_native_forms_are_handled() {
        let usage =
            openai_usage(&json!({ "prompt_tokens": 2000, "completion_tokens": 5, "prompt_tokens_details": { "cached_tokens": 1536 } }));
        assert_eq!(usage, Usage { input: 2000, output: 5, cached: 1536, cache_write: 0 });
        let p = OpenAICompatible::new("http://x/v1", None, "m");
        let body = p.request_body(&[json!({ "role": "assistant", "content": "hi", "native": [{ "type": "thinking" }] })], &[]);
        assert_eq!(body["messages"][0], json!({ "role": "assistant", "content": "hi" }));
    }

    #[test]
    fn an_error_in_the_stream_is_reported() {
        let err = StreamedReply::default().push_line(r#"data: {"error":{"message":"overloaded"}}"#).unwrap_err();
        assert!(matches!(err, ModelError::Fatal(m) if m.contains("overloaded")));
    }
}
