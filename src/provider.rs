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

/// Pure — unit-tested. Whether a model reads images, from its name, for
/// servers that don't say: Some(true) for families known to (Gemini, GPT-4o
/// and later, the o-series, Claude, Gemma 3 and 4, Llama 4, and any VL or
/// vision model), Some(false) for DeepSeek's text models, None otherwise.
pub fn known_to_see(model: &str) -> Option<bool> {
    let lower = model.to_ascii_lowercase();
    // "models/gemini-…", "google/gemini-…", "openai/gpt-5": the name after the last "/".
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let has = |part: &str| name.contains(part);
    if has("embed") || has("tts") || has("whisper") || has("image-gen") {
        return None;
    }
    let vision_marked = has("-vl")
        || has("vl-")
        || has("vl:")
        || name.ends_with("vl")
        || has("vision")
        || has("llava")
        || has("pixtral")
        || has("minicpm-v")
        || has("moondream");
    let families = ["gemini", "gpt-4o", "gpt-4.1", "gpt-5", "claude", "gemma3", "gemma-3", "gemma4", "gemma-4", "llama4", "llama-4"];
    let o_series = ["o1", "o3", "o4"].iter().any(|o| name == *o || name.starts_with(&format!("{o}-")));
    if vision_marked || o_series || families.iter().any(|f| name.starts_with(f) || has(&format!("-{f}"))) {
        return Some(true);
    }
    if name.starts_with("deepseek") {
        return Some(false);
    }
    None
}

/// What the person sees when they stop a request mid-answer.
pub const STOPPED: &str = "stopped";

/// Models whose server refused the reasoning-effort field ("address|model"),
/// for this run: a chat's default effort is no longer sent to them.
static REFUSED_EFFORT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn effort_key(base_url: &str, model: &str) -> String {
    format!("{}|{model}", base_url.trim_end_matches('/'))
}

/// Whether this model's server refused the effort field earlier in this run.
pub fn refused_effort(base_url: &str, model: &str) -> bool {
    REFUSED_EFFORT.lock().map(|list| list.contains(&effort_key(base_url, model))).unwrap_or(false)
}

/// Notes that this model's server refused the effort field.
pub fn note_refused_effort(base_url: &str, model: &str) {
    if let Ok(mut list) = REFUSED_EFFORT.lock() {
        let key = effort_key(base_url, model);
        if !list.contains(&key) {
            list.push(key);
        }
    }
}

/// Pure — unit-tested. Whether an answer can mean "this request has a field
/// the model doesn't take": a bad request (400) or an unprocessable one (422).
pub fn may_refuse_a_field(code: u16) -> bool {
    code == 400 || code == 422
}

pub struct OpenAICompatible {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// Sent as `reasoning_effort` when set (low / medium / high).
    pub effort: Option<String>,
    /// The effort is a default, not the person's choice: when the server
    /// answers 400/422, the request is tried once more without it, and on
    /// success the model gets none from then on.
    pub effort_optional: bool,
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
        let http = crate::http::builder().timeout_connect(Duration::from_secs(30)).timeout_read(Duration::from_secs(60)).build();
        OpenAICompatible {
            stall_base: crate::stall::base_for(base_url, false),
            clients: Default::default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model: model.to_string(),
            effort: None,
            effort_optional: false,
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
    /// `post`, and when the server refuses a default effort, once more
    /// without it (see `effort_optional`).
    fn send(&mut self, body: Value, read: Duration) -> Result<ureq::Response, ModelError> {
        let optional = self.effort_optional && body.get("reasoning_effort").is_some();
        match self.post_status(body.clone(), read) {
            Err((Some(code), error)) if optional && may_refuse_a_field(code) => {
                let mut without = body;
                if let Some(fields) = without.as_object_mut() {
                    fields.remove("reasoning_effort");
                }
                // Refused again: the effort was not the reason; say the first answer.
                let response = self.post(without, read).map_err(|_| error)?;
                self.effort = None;
                note_refused_effort(&self.base_url, &self.model);
                Ok(response)
            }
            other => other.map_err(|(_, error)| error),
        }
    }

    /// `read`: how long the server may send nothing.
    fn post(&self, body: Value, read: Duration) -> Result<ureq::Response, ModelError> {
        self.post_status(body, read).map_err(|(_, error)| error)
    }

    /// Like `post`, with the HTTP status of a refusal.
    fn post_status(&self, body: Value, read: Duration) -> Result<ureq::Response, (Option<u16>, ModelError)> {
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
                    Err((Some(code), ModelError::Retryable(message)))
                } else {
                    Err((Some(code), ModelError::Fatal(message)))
                }
            }
            Err(ureq::Error::Transport(t)) => Err((None, ModelError::Retryable(format!("could not reach {url}: {t}")))),
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

    /// Ollama lists a model's capabilities ("vision" means it reads images);
    /// other servers don't say, so the model's family decides when it is
    /// known (`known_to_see`).
    fn sees_images(&self) -> Option<bool> {
        let listed = self.ollama_show().and_then(|show| {
            let capabilities = show["capabilities"].as_array()?;
            Some(capabilities.iter().any(|c| c == "vision"))
        });
        listed.or_else(|| known_to_see(&self.model))
    }

    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError> {
        let body = self.request_body(messages, tools);
        let read = crate::stall::limit(crate::stall::WHOLE_ANSWER, body.to_string().len());
        let body: Value = self
            .send(body, read)?
            .into_json()
            .map_err(|e| ModelError::Retryable(format!("unreadable reply from the model server: {e}")))?;
        parse_reply(&body)
    }

    fn chat_stream(&mut self, messages: &[Value], tools: &[Value], on_text: &mut dyn FnMut(&str)) -> Result<Reply, ModelError> {
        let mut body = self.request_body(messages, tools);
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
        let read = crate::stall::limit(self.stall_base, body.to_string().len());
        let mut lines = std::io::BufReader::new(self.send(body, read)?.into_reader());
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
    fn model_families_known_to_read_images_are_recognised() {
        for sees in [
            "models/gemini-4-hydrogen",
            "gemini-2.5-flash",
            "google/gemini-3-pro",
            "gpt-4o-mini",
            "gpt-5",
            "openai/gpt-4.1",
            "o3",
            "o4-mini",
            "claude-sonnet-5-5",
            "gemma3:27b",
            "gemma4:31b",
            "qwen2.5vl:7b",
            "qwen3-vl-235b",
            "llama3.2-vision",
            "meta-llama/llama-4-maverick",
            "llava:13b",
            "pixtral-large",
            "deepseek-vl2",
        ] {
            assert_eq!(known_to_see(sees), Some(true), "{sees}");
        }
        for blind in ["deepseek-v4.1-flash", "deepseek-chat", "deepseek-reasoner"] {
            assert_eq!(known_to_see(blind), Some(false), "{blind}");
        }
        // Unknown, or not a chat model: the server decides.
        for unknown in ["kimi-k3", "glm-5.2", "qwen3-coder", "mistral-large", "gpt-oss:120b", "text-embedding-3-large", "olmo-3"] {
            assert_eq!(known_to_see(unknown), None, "{unknown}");
        }
    }

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

    /// A stand-in model server: refuses (400) any request that carries
    /// `reasoning_effort` when `refuses_effort`, answers the rest with "ok".
    /// Returns its address and, at the end, the bodies it was sent.
    fn effort_server(requests: usize, refuses_effort: bool, refuses_all: bool) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut seen = Vec::new();
                let mut buffer = [0u8; 8192];
                let body = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    seen.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&seen).to_string();
                    if let Some((head, sent)) = text.split_once("\r\n\r\n") {
                        let length: usize = head
                            .lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                            .unwrap_or(0);
                        if sent.len() >= length {
                            break serde_json::from_str::<Value>(sent).unwrap();
                        }
                    }
                };
                let refuse = refuses_all || (refuses_effort && body.get("reasoning_effort").is_some());
                let (status, reply) = if refuse {
                    ("400 Bad Request", json!({ "error": { "message": "unknown field" } }))
                } else {
                    ("200 OK", json!({ "choices": [{ "message": { "role": "assistant", "content": "ok" } }] }))
                };
                let reply = reply.to_string();
                let answer = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                stream.write_all(answer.as_bytes()).unwrap();
                bodies.push(body);
            }
            bodies
        });
        (url, handle)
    }

    fn effort_provider(url: &str, model: &str, optional: bool) -> OpenAICompatible {
        let mut p = OpenAICompatible::new(url, None, model);
        p.effort = Some("medium".into());
        p.effort_optional = optional;
        p
    }

    #[test]
    fn a_default_effort_the_server_refuses_is_dropped_and_remembered() {
        let (url, server) = effort_server(3, true, false);
        let mut p = effort_provider(&url, "no-effort-model", true);
        let hi = [json!({ "role": "user", "content": "hi" })];
        assert_eq!(p.chat(&hi, &[]).unwrap().content, "ok");
        assert!(p.effort.is_none() && refused_effort(&url, "no-effort-model"));
        assert_eq!(p.chat(&hi, &[]).unwrap().content, "ok", "the next request goes without it");
        let bodies = server.join().unwrap();
        assert_eq!(bodies[0]["reasoning_effort"], "medium");
        assert!(bodies[1].get("reasoning_effort").is_none() && bodies[2].get("reasoning_effort").is_none());
    }

    #[test]
    fn an_effort_the_person_chose_is_never_dropped() {
        let (url, server) = effort_server(1, true, false);
        let mut p = effort_provider(&url, "chosen-effort-model", false);
        let error = p.chat(&[json!({ "role": "user", "content": "hi" })], &[]).unwrap_err().to_string();
        assert!(error.contains("400") && error.contains("unknown field"), "{error}");
        assert_eq!(p.effort.as_deref(), Some("medium"));
        assert!(!refused_effort(&url, "chosen-effort-model"));
        assert_eq!(server.join().unwrap().len(), 1, "one request, no retry");
    }

    #[test]
    fn a_request_refused_with_or_without_the_effort_keeps_it_and_says_the_first_answer() {
        let (url, server) = effort_server(2, false, true);
        let mut p = effort_provider(&url, "broken-model", true);
        let error = p.chat(&[json!({ "role": "user", "content": "hi" })], &[]).unwrap_err().to_string();
        assert!(error.contains("400"), "{error}");
        assert_eq!(p.effort.as_deref(), Some("medium"), "not the effort's fault");
        assert!(!refused_effort(&url, "broken-model"));
        server.join().unwrap();
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
