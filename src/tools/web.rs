//! web_search: web search through the Tavily API (https://tavily.com).
//! Ported from sak's TavilySearchTool. The agent gets it only when a
//! TAVILY_API_KEY is set.

use super::{bool_arg, int_arg, str_arg, Tool, ToolResult};
use crate::text::one_line;
use serde_json::{json, Value};
use std::time::Duration;

pub const TAVILY_URL: &str = "https://api.tavily.com/search";

pub struct WebSearch {
    api_key: String,
    endpoint: String,
    http: ureq::Agent,
}

impl WebSearch {
    pub fn new(api_key: String) -> Self {
        Self::with_endpoint(api_key, TAVILY_URL)
    }

    /// For tests: another address than Tavily's.
    pub fn with_endpoint(api_key: String, endpoint: &str) -> Self {
        let http = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
        WebSearch { api_key, endpoint: endpoint.to_string(), http }
    }
}

impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web and return the most relevant pages with a short extract of each, plus a direct answer when one \
         is available. Use it for current events, docs, error messages, library versions — anything you don't know or \
         that may have changed."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "What to search for, phrased like a search-engine query." },
                "max_results": { "type": "integer", "description": "How many results to return, 1–10. Default 5." },
                "topic": { "type": "string", "description": "`general` (default) or `news` for recent events." },
                "deep": { "type": "boolean", "description": "Slower, more thorough search (costs 2 credits instead of 1). Default false." }
            },
            "required": ["query"]
        })
    }
    // Only reads from the web: no approval needed, like read_file.

    fn run(&self, args: &Value) -> ToolResult {
        let Some(query) = str_arg(args, "query").map(str::trim).filter(|q| !q.is_empty()) else {
            return ToolResult::err("web_search requires a non-empty `query`.");
        };
        let body = request_body(query, int_arg(args, "max_results"), str_arg(args, "topic"), bool_arg(args, "deep").unwrap_or(false));
        // No retry: each search spends a credit.
        let response = self.http.post(&self.endpoint).set("Authorization", &format!("Bearer {}", self.api_key)).send_json(body);
        match response {
            Ok(response) => match response.into_json::<Value>() {
                Ok(json) => ToolResult::ok(render(query, &json)),
                Err(_) => ToolResult::err("Could not read Tavily's response."),
            },
            Err(ureq::Error::Status(code, response)) => {
                let hint = match code {
                    401 => " (check TAVILY_API_KEY)",
                    429 => " (rate limited — wait and retry)",
                    432 | 433 => " (Tavily plan or credit limit reached)",
                    _ => "",
                };
                ToolResult::err(format!(
                    "Tavily returned HTTP {code}{hint}: {}",
                    one_line(&response.into_string().unwrap_or_default(), 300)
                ))
            }
            Err(ureq::Error::Transport(t)) => ToolResult::err(format!("web_search failed: {t}")),
        }
    }
}

/// Pure — unit-tested.
pub fn request_body(query: &str, max_results: Option<i64>, topic: Option<&str>, deep: bool) -> Value {
    json!({
        "query": query,
        "max_results": max_results.unwrap_or(5).clamp(1, 10),
        "topic": if topic == Some("news") { "news" } else { "general" },
        "search_depth": if deep { "advanced" } else { "basic" },
        "include_answer": true,
    })
}

/// Pure — unit-tested. Tavily's answer as text for the model: the direct
/// answer, then each result's title, address and up to 800 characters.
pub fn render(query: &str, json: &Value) -> String {
    let mut out = format!("Search: {query}\n");
    if let Some(answer) = json["answer"].as_str().filter(|a| !a.is_empty()) {
        out.push_str(&format!("\nAnswer: {answer}\n"));
    }
    let results = json["results"].as_array().cloned().unwrap_or_default();
    if results.is_empty() {
        out.push_str("\nNo results.");
        return out;
    }
    for (i, result) in results.iter().enumerate() {
        let url = result["url"].as_str().unwrap_or("");
        let title = result["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(url);
        out.push_str(&format!("\n[{}] {title}\n{url}\n", i + 1));
        if let Some(content) = result["content"].as_str().filter(|c| !c.is_empty()) {
            out.push_str(&content.chars().take(800).collect::<String>());
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn the_request_is_clamped_and_defaults_are_filled_in() {
        let body = request_body("rust 1.95 release", None, None, false);
        assert_eq!(
            body,
            json!({ "query": "rust 1.95 release", "max_results": 5, "topic": "general", "search_depth": "basic", "include_answer": true })
        );
        let body = request_body("q", Some(50), Some("news"), true);
        assert_eq!(
            (body["max_results"].as_i64(), body["topic"].as_str(), body["search_depth"].as_str()),
            (Some(10), Some("news"), Some("advanced"))
        );
        assert_eq!(request_body("q", Some(0), Some("sports"), false)["max_results"], 1);
        assert_eq!(request_body("q", None, Some("sports"), false)["topic"], "general");
    }

    #[test]
    fn results_are_rendered_with_the_answer_first_and_long_extracts_cut() {
        let json = json!({ "answer": "1.95.0", "results": [
            { "title": "Announcing Rust 1.95", "url": "https://blog.rust-lang.org/x", "content": "x".repeat(2000) },
            { "title": "", "url": "https://example.com/y", "content": "" } ] });
        let text = render("rust release", &json);
        assert!(text.starts_with("Search: rust release\n\nAnswer: 1.95.0\n\n[1] Announcing Rust 1.95\nhttps://blog.rust-lang.org/x\n"));
        assert!(text.contains(&format!("{}\n", "x".repeat(800))) && !text.contains(&"x".repeat(801)));
        assert!(text.contains("[2] https://example.com/y\nhttps://example.com/y\n"), "no title: the address stands in");
        assert_eq!(render("q", &json!({ "results": [] })), "Search: q\n\nNo results.");
    }

    /// A stand-in for Tavily: answers one request and returns what it was sent.
    fn serve_once(status: &'static str, body: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/search", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                seen.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some((head, sent)) = text.split_once("\r\n\r\n") {
                    let length: usize = head
                        .lines()
                        .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    if sent.len() >= length {
                        break;
                    }
                }
            }
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&seen).to_string()
        });
        (url, handle)
    }

    #[test]
    fn a_search_sends_the_key_and_returns_the_rendered_results() {
        let (url, server) = serve_once(
            "200 OK",
            json!({ "answer": "Paris", "results": [{ "title": "France", "url": "https://x/fr", "content": "Capital: Paris." }] })
                .to_string(),
        );
        let tool = WebSearch::with_endpoint("tvly-test".into(), &url);
        assert!(!tool.needs_approval(), "reading the web needs no approval");
        let result = tool.run(&json!({ "query": " capital of France ", "max_results": 3 }));
        assert!(result.ok && result.text.contains("Answer: Paris") && result.text.contains("[1] France"), "{}", result.text);
        let request = server.join().unwrap();
        assert!(request.to_lowercase().contains("authorization: bearer tvly-test"));
        assert!(request.contains("\"query\":\"capital of France\"") && request.contains("\"max_results\":3"), "{request}");
    }

    #[test]
    fn errors_say_what_to_do() {
        let (url, server) = serve_once("401 Unauthorized", json!({ "detail": "invalid key" }).to_string());
        let result = WebSearch::with_endpoint("bad".into(), &url).run(&json!({ "query": "x" }));
        server.join().unwrap();
        assert!(
            !result.ok && result.text.contains("HTTP 401 (check TAVILY_API_KEY)") && result.text.contains("invalid key"),
            "{}",
            result.text
        );
        assert!(!WebSearch::new("k".into()).run(&json!({ "query": "  " })).ok);
    }
}
