//! How long a model call may go without a byte arriving before it counts as
//! stalled. From SwiftAgentKit's `StreamStallPolicy` (itself after Hermes):
//! the limit starts from where the model runs — a model on this machine reads
//! a long prompt slowly; a cloud model that thinks silently sends nothing for
//! a while; Anthropic's stream carries pings — and grows with the prompt,
//! because reading a large prompt delays the first token.
//!
//! A stalled call fails as one worth retrying (and falling back from), instead
//! of waiting out one fixed limit for every server.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// Until the whole answer arrives at once (no streaming), nothing arrives at
/// all: the most patient limit.
pub const WHOLE_ANSWER: Duration = Duration::from_secs(900);

/// Pure — unit-tested. The starting limit for a server at `base_url`.
pub fn base_for(base_url: &str, anthropic: bool) -> Duration {
    let host = base_url.split("://").nth(1).unwrap_or(base_url).split(['/', ':']).next().unwrap_or("");
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0") || host.ends_with(".local");
    Duration::from_secs(if local {
        900
    } else if anthropic {
        300
    } else {
        600
    })
}

/// Pure — unit-tested. The limit for one call whose request body is
/// `body_chars` long: +60 s past about 50K tokens, +120 s past about 100K.
pub fn limit(base: Duration, body_chars: usize) -> Duration {
    let tokens = body_chars / 4;
    base + Duration::from_secs(if tokens > 100_000 {
        120
    } else if tokens > 50_000 {
        60
    } else {
        0
    })
}

/// HTTP clients by read limit: ureq sets that limit per client, and a client
/// keeps its connections open, so one per limit in use is kept.
#[derive(Default)]
pub struct Clients {
    by_limit: Mutex<HashMap<u64, ureq::Agent>>,
}

impl Clients {
    pub fn with_read_limit(&self, read: Duration) -> ureq::Agent {
        let mut clients = self.by_limit.lock().unwrap();
        clients
            .entry(read.as_secs())
            .or_insert_with(|| ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(30)).timeout_read(read).build())
            .clone()
    }
}

/// Pure — unit-tested. The words for an answer that stopped arriving: a
/// stall says how long nothing came.
pub fn stopped_arriving(error: &std::io::Error, read: Duration) -> String {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => format!("nothing arrived for {} s", read.as_secs()),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_starts_from_where_the_model_runs_and_grows_with_the_prompt() {
        assert_eq!(base_for("http://localhost:11434/v1", false).as_secs(), 900);
        assert_eq!(base_for("http://127.0.0.1:8080/v1", false).as_secs(), 900);
        assert_eq!(base_for("https://api.anthropic.com", true).as_secs(), 300);
        assert_eq!(base_for("https://ollama.com/v1", false).as_secs(), 600);
        let base = Duration::from_secs(300);
        assert_eq!(limit(base, 40_000).as_secs(), 300);
        assert_eq!(limit(base, 4 * 60_000).as_secs(), 360);
        assert_eq!(limit(base, 4 * 150_000).as_secs(), 420);
    }

    #[test]
    fn a_stall_says_how_long_nothing_came() {
        let timed_out = std::io::Error::new(std::io::ErrorKind::TimedOut, "x");
        assert_eq!(stopped_arriving(&timed_out, Duration::from_secs(600)), "nothing arrived for 600 s");
        let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
        assert_eq!(stopped_arriving(&reset, Duration::from_secs(600)), "connection reset");
    }

    /// A server that sends the start of a stream, then nothing: the call
    /// fails at the limit as one worth retrying, saying how long it waited.
    #[test]
    fn a_silent_stream_fails_at_its_limit() {
        use crate::provider::{Model, ModelError, OpenAICompatible};
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 65536];
            let _ = socket.read(&mut buf);
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n: hello\n\n");
            std::thread::sleep(Duration::from_secs(4));
        });
        let mut model = OpenAICompatible::new(&format!("http://127.0.0.1:{port}/v1"), None, "m");
        model.stall_base = Duration::from_secs(1);
        let started = std::time::Instant::now();
        let error = model.chat_stream(&[serde_json::json!({ "role": "user", "content": "hi" })], &[], &mut |_| {}).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(3), "gave up at the limit, not later");
        match error {
            ModelError::Retryable(text) => assert!(text.contains("nothing arrived for 1 s"), "{text}"),
            other => panic!("{other:?}"),
        }
    }
}
