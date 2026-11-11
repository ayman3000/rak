//! A connection to one server: started as a program (stdin/stdout) or
//! reached at a web address, then spoken to in JSON-RPC.

use super::config::{ServerConfig, Transport};
use crate::interrupt::Stop;
use crate::text::one_line;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const PROTOCOL: &str = "2025-06-18";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// A tool result longer than this is cut: one web page must not fill the context.
pub(crate) const MAX_RESULT_CHARS: usize = 20_000;
/// A tool description longer than this is cut: it is sent with every request.
pub(crate) const MAX_DESCRIPTION_CHARS: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments.
    pub schema: Value,
    /// The server says this tool changes nothing.
    pub read_only: bool,
}

enum Conn {
    Stdio {
        child: Child,
        stdin: ChildStdin,
        lines: Receiver<String>,
        /// The last of what the program wrote to stderr, for error messages.
        errors: Arc<Mutex<String>>,
    },
    Http {
        http: ureq::Agent,
        url: String,
        headers: BTreeMap<String, String>,
        session: Option<String>,
        /// After `initialize`: the protocol version goes on every request.
        ready: bool,
    },
}

struct State {
    conn: Conn,
    next_id: u64,
}

/// A connection to one server. Dropping it stops the server's program.
pub struct Client {
    pub name: String,
    state: Mutex<State>,
    project: PathBuf,
    stop: Stop,
    call_timeout: Duration,
}

impl Client {
    /// Starts (or reaches) the server and completes the protocol's handshake.
    pub fn connect(config: &ServerConfig, project: &Path, stop: &Stop) -> Result<Client, String> {
        let conn = match &config.transport {
            Transport::Stdio { command, args, env } => start_program(command, args, env, project)?,
            Transport::Http { url, headers } => Conn::Http {
                http: ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).build(),
                url: url.clone(),
                headers: headers.clone(),
                session: None,
                ready: false,
            },
        };
        let client = Client {
            name: config.name.clone(),
            state: Mutex::new(State { conn, next_id: 0 }),
            project: project.to_path_buf(),
            stop: stop.clone(),
            call_timeout: config.timeout.map(Duration::from_secs).unwrap_or(CALL_TIMEOUT),
        };
        let hello = json!({
            "protocolVersion": PROTOCOL,
            "capabilities": { "roots": { "listChanged": false } },
            "clientInfo": { "name": "rak", "version": env!("CARGO_PKG_VERSION") },
        });
        client.request("initialize", hello, CONNECT_TIMEOUT)?;
        if let Conn::Http { ready, .. } = &mut client.state.lock().unwrap().conn {
            *ready = true;
        }
        client.notify("notifications/initialized", json!({}));
        Ok(client)
    }

    /// Every tool the server offers.
    pub fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = cursor.as_ref().map(|c| json!({ "cursor": c })).unwrap_or_else(|| json!({}));
            let page = self.request("tools/list", params, CONNECT_TIMEOUT)?;
            tools.extend(page["tools"].as_array().into_iter().flatten().filter_map(tool_info));
            cursor = page["nextCursor"].as_str().filter(|c| !c.is_empty()).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// Runs a tool. Ok is (the server called it an error, its text); Err is
    /// a failure to get any answer.
    pub fn call_tool(&self, tool: &str, arguments: &Value) -> Result<(bool, String), String> {
        let result = self.request("tools/call", json!({ "name": tool, "arguments": arguments }), self.call_timeout)?;
        Ok((result["isError"] == true, result_text(&result)))
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let mut state = self.state.lock().unwrap();
        state.next_id += 1;
        let id = state.next_id;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let reply = match &mut state.conn {
            Conn::Stdio { stdin, lines, errors, .. } => self.ask_program(stdin, lines, errors, &message, id, timeout)?,
            Conn::Http { http, url, headers, session, ready } => {
                ask_address(http, url, headers, session, *ready, &message, Some(id), timeout)?
            }
        };
        match reply.get("error").filter(|e| !e.is_null()) {
            Some(error) => Err(error["message"].as_str().map(str::to_string).unwrap_or_else(|| error.to_string())),
            None => Ok(reply["result"].clone()),
        }
    }

    fn notify(&self, method: &str, params: Value) {
        let mut state = self.state.lock().unwrap();
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &mut state.conn {
            Conn::Stdio { stdin, .. } => {
                let _ = writeln!(stdin, "{message}").and_then(|_| stdin.flush());
            }
            Conn::Http { http, url, headers, session, ready } => {
                let _ = ask_address(http, url, headers, session, *ready, &message, None, Duration::from_secs(15));
            }
        }
    }

    fn ask_program(
        &self,
        stdin: &mut ChildStdin,
        lines: &Receiver<String>,
        errors: &Arc<Mutex<String>>,
        message: &Value,
        id: u64,
        timeout: Duration,
    ) -> Result<Value, String> {
        let gone = || {
            let said = errors.lock().unwrap().trim().to_string();
            if said.is_empty() {
                "the server's program stopped".to_string()
            } else {
                format!("the server's program stopped: {}", one_line(&said, 300))
            }
        };
        if writeln!(stdin, "{message}").and_then(|_| stdin.flush()).is_err() {
            // Give its last words a moment to arrive.
            std::thread::sleep(Duration::from_millis(100));
            return Err(gone());
        }
        let give_up = |stdin: &mut ChildStdin, why: String| {
            let cancel = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": id } });
            let _ = writeln!(stdin, "{cancel}").and_then(|_| stdin.flush());
            Err(why)
        };
        let deadline = Instant::now() + timeout;
        loop {
            if self.stop.requested() {
                return give_up(stdin, "stopped by the user".to_string());
            }
            if Instant::now() >= deadline {
                return give_up(stdin, format!("no answer within {} s", timeout.as_secs()));
            }
            let line = match lines.recv_timeout(Duration::from_millis(50)) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    std::thread::sleep(Duration::from_millis(100));
                    return Err(gone());
                }
            };
            // Anything that isn't a protocol message (a log line) is skipped.
            let Ok(incoming) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(method) = incoming["method"].as_str() {
                // The server asking us something. Notifications need no answer.
                if let Some(their_id) = incoming.get("id").filter(|i| !i.is_null()) {
                    let answer = self.answer_server(method, their_id);
                    let _ = writeln!(stdin, "{answer}").and_then(|_| stdin.flush());
                }
            } else if incoming["id"] == json!(id) {
                return Ok(incoming);
            }
        }
    }

    /// What rak says when a server asks it something.
    fn answer_server(&self, method: &str, id: &Value) -> Value {
        match method {
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "roots/list" => {
                let name = self.project.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let root = json!({ "uri": format!("file://{}", self.project.display()), "name": name });
                json!({ "jsonrpc": "2.0", "id": id, "result": { "roots": [root] } })
            }
            _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("rak does not support {method}") } }),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else { return };
        if let Conn::Stdio { child, .. } = &mut state.conn {
            stop_program(child);
        }
    }
}

fn start_program(command: &str, args: &[String], env: &BTreeMap<String, String>, project: &Path) -> Result<Conn, String> {
    let mut program = Command::new(command);
    program.args(args).envs(env).current_dir(project).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Its own process group: Ctrl-C at the terminal stops the request, not the server.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        program.process_group(0);
    }
    let mut child = program.spawn().map_err(|e| format!("couldn't start `{command}`: {e}"))?;
    let stdin = child.stdin.take().ok_or("couldn't open the program's input")?;
    let stdout = child.stdout.take().ok_or("couldn't open the program's output")?;
    let stderr = child.stderr.take().ok_or("couldn't open the program's error output")?;

    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let errors = Arc::new(Mutex::new(String::new()));
    let kept = errors.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut tail = kept.lock().unwrap();
            tail.push_str(&line);
            tail.push('\n');
            if tail.len() > 4_000 {
                let cut = tail.len() - 2_000;
                let cut = (cut..tail.len()).find(|i| tail.is_char_boundary(*i)).unwrap_or(tail.len());
                tail.drain(..cut);
            }
        }
    });
    Ok(Conn::Stdio { child, stdin, lines, errors })
}

/// Asks the program's whole process group to end, then makes sure.
fn stop_program(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    for _ in 0..30 {
        if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// One message to a server at a web address. With an `id`, waits for the
/// reply to it: the body itself, or an event stream that carries it.
#[allow(clippy::too_many_arguments)]
fn ask_address(
    http: &ureq::Agent,
    url: &str,
    headers: &BTreeMap<String, String>,
    session: &mut Option<String>,
    ready: bool,
    message: &Value,
    id: Option<u64>,
    timeout: Duration,
) -> Result<Value, String> {
    let mut request =
        http.post(url).timeout(timeout).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream");
    if ready {
        request = request.set("MCP-Protocol-Version", PROTOCOL);
    }
    if let Some(session) = session.as_deref() {
        request = request.set("Mcp-Session-Id", session);
    }
    for (name, value) in headers {
        request = request.set(name, value);
    }
    let response = match request.send_string(&message.to_string()) {
        Ok(response) => response,
        Err(ureq::Error::Status(401 | 403, _)) => {
            return Err(format!("{url} needs a sign-in. rak cannot sign in yet; a key can go in a header (--header)"))
        }
        Err(ureq::Error::Status(code, response)) => {
            return Err(format!("{url} answered {code}: {}", one_line(&response.into_string().unwrap_or_default(), 200)));
        }
        Err(ureq::Error::Transport(t)) => return Err(format!("could not reach {url}: {t}")),
    };
    if let Some(given) = response.header("Mcp-Session-Id") {
        *session = Some(given.to_string());
    }
    let Some(id) = id else { return Ok(Value::Null) };
    let is_ours = |v: &Value| v["id"] == json!(id) && v.get("method").is_none();
    if response.content_type() == "text/event-stream" {
        for line in BufReader::new(response.into_reader()).lines().map_while(Result::ok) {
            let Some(data) = line.strip_prefix("data:") else { continue };
            if let Some(reply) = serde_json::from_str::<Value>(data.trim()).ok().filter(is_ours) {
                return Ok(reply);
            }
        }
        return Err(format!("{url} ended its answer without the reply"));
    }
    let body: Value = response.into_json().map_err(|e| format!("unreadable answer from {url}: {e}"))?;
    Some(body).filter(is_ours).ok_or_else(|| format!("{url} answered something else than the reply"))
}

/// Pure — unit-tested. One entry of `tools/list`.
pub fn tool_info(tool: &Value) -> Option<ToolInfo> {
    let name = tool["name"].as_str().filter(|n| !n.is_empty())?.to_string();
    let mut schema = match &tool["inputSchema"] {
        Value::Object(schema) => Value::Object(schema.clone()),
        _ => json!({}),
    };
    // Model servers want an object schema with its properties named.
    schema["type"] = json!("object");
    if !schema["properties"].is_object() {
        schema["properties"] = json!({});
    }
    let description = tool["description"].as_str().unwrap_or("").trim();
    let description = if description.chars().count() > MAX_DESCRIPTION_CHARS {
        description.chars().take(MAX_DESCRIPTION_CHARS).collect::<String>() + "…"
    } else {
        description.to_string()
    };
    Some(ToolInfo { name, description, schema, read_only: tool["annotations"]["readOnlyHint"] == true })
}

/// Pure — unit-tested. A `tools/call` result as text for the model.
pub fn result_text(result: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for item in result["content"].as_array().into_iter().flatten() {
        let part = match item["type"].as_str().unwrap_or("") {
            "text" => item["text"].as_str().unwrap_or("").to_string(),
            "image" | "audio" => {
                format!("[{} not shown: {}]", item["type"].as_str().unwrap_or(""), item["mimeType"].as_str().unwrap_or("unknown type"))
            }
            "resource" => match item["resource"]["text"].as_str() {
                Some(text) => text.to_string(),
                None => format!("[resource {}]", item["resource"]["uri"].as_str().unwrap_or("")),
            },
            "resource_link" => format!("[link {}]", item["uri"].as_str().unwrap_or("")),
            _ => continue,
        };
        if !part.is_empty() {
            parts.push(part);
        }
    }
    let mut text = parts.join("\n");
    if text.is_empty() {
        if let Some(structured) = result.get("structuredContent").filter(|s| !s.is_null()) {
            text = structured.to_string();
        }
    }
    if text.is_empty() {
        return "(the tool returned nothing)".to_string();
    }
    let length = text.chars().count();
    if length > MAX_RESULT_CHARS {
        text = text.chars().take(MAX_RESULT_CHARS).collect::<String>()
            + &format!("\n[cut: {} more characters not shown]", length - MAX_RESULT_CHARS);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_entries_become_tools_with_an_object_schema() {
        let info = tool_info(&json!({ "name": "search", "description": " Find things. ", "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } }, "required": ["q"] } }))
        .unwrap();
        assert_eq!((info.name.as_str(), info.description.as_str(), info.read_only), ("search", "Find things.", true));
        assert_eq!(info.schema["required"][0], "q");
        let bare = tool_info(&json!({ "name": "now" })).unwrap();
        assert_eq!(bare.schema, json!({ "type": "object", "properties": {} }));
        assert!(!bare.read_only, "no hint: it asks");
        assert!(tool_info(&json!({ "description": "nameless" })).is_none());
        let long = tool_info(&json!({ "name": "x", "description": "d".repeat(5_000) })).unwrap();
        assert_eq!(long.description.chars().count(), MAX_DESCRIPTION_CHARS + 1);
    }

    #[test]
    fn results_become_text_and_long_ones_are_cut() {
        let result = json!({ "content": [
            { "type": "text", "text": "first" },
            { "type": "image", "data": "…", "mimeType": "image/png" },
            { "type": "resource", "resource": { "uri": "file:///a", "text": "inside" } },
            { "type": "resource_link", "uri": "file:///b" } ] });
        assert_eq!(result_text(&result), "first\n[image not shown: image/png]\ninside\n[link file:///b]");
        assert_eq!(result_text(&json!({ "content": [], "structuredContent": { "n": 1 } })), "{\"n\":1}");
        assert_eq!(result_text(&json!({ "content": [] })), "(the tool returned nothing)");
        let long = result_text(&json!({ "content": [{ "type": "text", "text": "x".repeat(MAX_RESULT_CHARS + 500) }] }));
        assert!(long.ends_with("[cut: 500 more characters not shown]"));
    }
}
