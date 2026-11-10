//! The model side: one trait the agent loop talks to, and a client for any
//! server that speaks OpenAI's chat-completions API (OpenAI, OpenRouter,
//! Ollama, Hugging Face's router, vLLM, llama.cpp, LM Studio…).

use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
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
}

pub struct OpenAICompatible {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// Sent as `reasoning_effort` when set (low / medium / high).
    pub effort: Option<String>,
    /// Sent as `max_tokens` when set; otherwise the server's default applies.
    pub max_tokens: Option<u64>,
    http: ureq::Agent,
}

impl OpenAICompatible {
    pub fn new(base_url: &str, api_key: Option<String>, model: &str) -> Self {
        let http = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(30))
            .timeout_read(Duration::from_secs(900))
            .build();
        OpenAICompatible {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model: model.to_string(),
            effort: None,
            max_tokens: None,
            http,
        }
    }

    /// Pure — unit-tested.
    pub fn request_body(&self, messages: &[Value], tools: &[Value]) -> Value {
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

impl Model for OpenAICompatible {
    fn chat(&mut self, messages: &[Value], tools: &[Value]) -> Result<Reply, ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self.http.post(&url);
        if let Some(key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        match request.send_json(self.request_body(messages, tools)) {
            Ok(response) => {
                let body: Value = response
                    .into_json()
                    .map_err(|e| ModelError::Retryable(format!("unreadable reply from the model server: {e}")))?;
                parse_reply(&body)
            }
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                let message = format!("the model server answered {code}: {}", one_line(&text, 300));
                if code == 408 || code == 429 || code >= 500 {
                    Err(ModelError::Retryable(message))
                } else {
                    Err(ModelError::Fatal(message))
                }
            }
            Err(ureq::Error::Transport(t)) => Err(ModelError::Retryable(format!("could not reach {url}: {t}"))),
        }
    }
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
    let usage = Usage {
        input: body.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
        output: body.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
    };
    Ok(Reply { content, tool_calls, usage })
}

pub fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        flat.chars().take(max).collect::<String>() + "…"
    } else {
        flat
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
        assert_eq!(reply.usage, Usage { input: 120, output: 9 });
    }

    #[test]
    fn null_content_is_empty_text() {
        let reply = parse_reply(&json!({ "choices": [{ "message": { "content": null } }] })).unwrap();
        assert_eq!(reply.content, "");
        assert!(reply.tool_calls.is_empty());
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
}
