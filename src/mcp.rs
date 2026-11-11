//! MCP client: tools from outside rak. A server is a separate program (or a
//! web address) that speaks the Model Context Protocol; its tools join the
//! agent's own. Servers are listed in `mcp.json`, in the shape Claude Code,
//! Claude Desktop and Cursor use, so an entry can be pasted across:
//!
//!   { "mcpServers": {
//!       "files": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] },
//!       "docs":  { "url": "https://example.com/mcp" } } }
//!
//! A tool the server marks read-only runs like read_file; any other asks
//! first, like edit_file. Read-only mode keeps only the read-only ones. Every
//! server adds its tool descriptions to every request — a cost worth knowing.

use crate::interrupt::Stop;
use crate::provider::one_line;
use crate::tools::{Tool, ToolResult};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROTOCOL: &str = "2025-06-18";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// A tool result longer than this is cut: one web page must not fill the context.
const MAX_RESULT_CHARS: usize = 20_000;
/// A tool description longer than this is cut: it is sent with every request.
const MAX_DESCRIPTION_CHARS: usize = 1_000;

// MARK: - The config file

#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    /// A program rak starts and talks to over its stdin and stdout.
    Stdio { command: String, args: Vec<String>, env: BTreeMap<String, String> },
    /// A server at a web address ("streamable HTTP").
    Http { url: String, headers: BTreeMap<String, String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub name: String,
    pub transport: Transport,
    /// Seconds one tool call may take. Default 120.
    pub timeout: Option<u64>,
}

fn strings(value: &Value) -> BTreeMap<String, String> {
    value.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()
}

/// Pure — unit-tested. The file's text → its servers, in name order. Entries
/// marked `"disabled": true` are left out.
pub fn parse_config(text: &str) -> Result<Vec<ServerConfig>, String> {
    let top: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON ({e})"))?;
    let servers = top.get("mcpServers").or_else(|| top.get("servers")).and_then(Value::as_object).ok_or("no \"mcpServers\" object")?;
    let mut out = Vec::new();
    for (name, entry) in servers {
        if entry["disabled"] == true || entry["enabled"] == false {
            continue;
        }
        let command = entry["command"].as_str().map(str::trim).filter(|c| !c.is_empty());
        let url = entry.get("url").or_else(|| entry.get("endpoint")).and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty());
        let transport = match (command, url) {
            (Some(command), _) => Transport::Stdio {
                command: command.to_string(),
                args: entry["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect(),
                env: strings(&entry["env"]),
            },
            (None, Some(url)) => Transport::Http { url: url.to_string(), headers: strings(&entry["headers"]) },
            (None, None) => return Err(format!("server \"{name}\" needs a \"command\" or a \"url\"")),
        };
        out.push(ServerConfig { name: name.clone(), transport, timeout: entry["timeout"].as_u64() });
    }
    Ok(out)
}

/// Pure — unit-tested. One server as its `mcp.json` entry.
pub fn entry_json(server: &ServerConfig) -> Value {
    let mut entry = match &server.transport {
        Transport::Stdio { command, args, env } => {
            let mut e = json!({ "command": command, "args": args });
            if !env.is_empty() {
                e["env"] = json!(env);
            }
            e
        }
        Transport::Http { url, headers } => {
            let mut e = json!({ "url": url });
            if !headers.is_empty() {
                e["headers"] = json!(headers);
            }
            e
        }
    };
    if let Some(seconds) = server.timeout {
        entry["timeout"] = json!(seconds);
    }
    entry
}

/// Where servers are listed, the project's file first: `./.rak/mcp.json`,
/// then `mcp.json` in rak's config folder.
pub fn config_files(project: &Path, config_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut files = vec![project.join(".rak").join("mcp.json")];
    files.extend(config_dir.map(|dir| dir.join("mcp.json")));
    files
}

/// The servers from every file that exists; a name in an earlier file wins.
/// A file that cannot be read is a line in the second list, not a reason to stop.
pub fn load(files: &[PathBuf]) -> (Vec<(ServerConfig, PathBuf)>, Vec<String>) {
    let (mut servers, mut problems): (Vec<(ServerConfig, PathBuf)>, Vec<String>) = (Vec::new(), Vec::new());
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        match parse_config(&text) {
            Ok(found) => {
                for server in found {
                    if !servers.iter().any(|(s, _)| s.name == server.name) {
                        servers.push((server, file.clone()));
                    }
                }
            }
            Err(why) => problems.push(format!("{}: {why}", file.display())),
        }
    }
    (servers, problems)
}

fn read_file_object(file: &Path) -> Result<Map<String, Value>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) if text.trim().is_empty() => Ok(Map::new()),
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(top)) => Ok(top),
            _ => Err(format!("{} is not a JSON object; fix or remove it first", file.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("cannot read {}: {e}", file.display())),
    }
}

fn write_file_object(file: &Path, top: Map<String, Value>) -> Result<(), String> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(top)).unwrap_or_default() + "\n";
    std::fs::write(file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    // The file can hold keys (env, headers): readable by its owner only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Adds the server to the file, or replaces the entry with its name. The rest
/// of the file is kept. Returns whether an entry was replaced.
pub fn add_to_file(file: &Path, server: &ServerConfig) -> Result<bool, String> {
    let mut top = read_file_object(file)?;
    let servers = top.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| format!("\"mcpServers\" in {} is not an object", file.display()))?;
    let replaced = servers.insert(server.name.clone(), entry_json(server)).is_some();
    write_file_object(file, top)?;
    Ok(replaced)
}

/// Removes the server from the file. Returns whether it was there.
pub fn remove_from_file(file: &Path, name: &str) -> Result<bool, String> {
    let mut top = read_file_object(file)?;
    let removed = top.get_mut("mcpServers").and_then(Value::as_object_mut).is_some_and(|servers| servers.remove(name).is_some());
    if removed {
        write_file_object(file, top)?;
    }
    Ok(removed)
}

// MARK: - One server

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
            Conn::Http { http, url, headers, session, ready } => ask_address(http, url, headers, session, *ready, &message, Some(id), timeout)?,
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

    fn ask_program(&self, stdin: &mut ChildStdin, lines: &Receiver<String>, errors: &Arc<Mutex<String>>, message: &Value, id: u64, timeout: Duration) -> Result<Value, String> {
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
fn ask_address(http: &ureq::Agent, url: &str, headers: &BTreeMap<String, String>, session: &mut Option<String>, ready: bool, message: &Value, id: Option<u64>, timeout: Duration) -> Result<Value, String> {
    let mut request = http.post(url).timeout(timeout).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream");
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
        Err(ureq::Error::Status(401 | 403, _)) => return Err(format!("{url} needs a sign-in. rak cannot sign in yet; a key can go in a header (--header)")),
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
            "image" | "audio" => format!("[{} not shown: {}]", item["type"].as_str().unwrap_or(""), item["mimeType"].as_str().unwrap_or("unknown type")),
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
        text = text.chars().take(MAX_RESULT_CHARS).collect::<String>() + &format!("\n[cut: {} more characters not shown]", length - MAX_RESULT_CHARS);
    }
    text
}

// MARK: - As agent tools

/// A server's tool as one of the agent's.
pub struct McpTool {
    client: Arc<Client>,
    /// The name the model sees: `server__tool`.
    exposed: String,
    info: ToolInfo,
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.exposed
    }
    fn description(&self) -> &str {
        &self.info.description
    }
    fn parameters(&self) -> Value {
        self.info.schema.clone()
    }
    fn needs_approval(&self) -> bool {
        !self.info.read_only
    }
    fn run(&self, args: &Value) -> ToolResult {
        match self.client.call_tool(&self.info.name, args) {
            Ok((false, text)) => ToolResult::ok(text),
            Ok((true, text)) => ToolResult::err(text),
            Err(why) => ToolResult::err(format!("The {} server gave no result: {why}.", self.client.name)),
        }
    }
}

/// Pure — unit-tested. The name a model sees for a server's tool: letters,
/// digits, `_` and `-` only, at most 64 characters (what model servers accept).
pub fn exposed_name(server: &str, tool: &str) -> String {
    let clean = |text: &str| -> String { text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect() };
    let mut name = format!("{}__{}", clean(server), clean(tool));
    name.truncate(64);
    name
}

/// What `connect_all` found.
#[derive(Default)]
pub struct Connected {
    pub tools: Vec<Box<dyn Tool>>,
    /// (server, how many of its tools joined), for the ones that connected.
    pub servers: Vec<(String, usize)>,
    /// One line per thing that went wrong.
    pub problems: Vec<String>,
    /// Tool name → the server it came from (for "allow all of this server").
    pub server_of: HashMap<String, String>,
    /// Tools left out because the run is read-only and they aren't.
    pub left_out: usize,
}

impl Connected {
    /// "7 MCP tools from files, docs" — nothing when no server connected.
    pub fn summary(&self) -> Option<String> {
        if self.servers.is_empty() {
            return None;
        }
        let names: Vec<&str> = self.servers.iter().map(|(name, _)| name.as_str()).collect();
        let count = self.tools.len();
        let note = if self.left_out > 0 { format!(" (read-only ones; {} left out)", self.left_out) } else { String::new() };
        Some(format!("{count} MCP tool{} from {}{note}", if count == 1 { "" } else { "s" }, names.join(", ")))
    }
}

/// Connects every server, all at once, and gathers their tools. A server
/// that fails is a line in `problems`, not a reason to stop.
pub fn connect_all(servers: &[ServerConfig], project: &Path, read_only: bool, stop: &Stop) -> Connected {
    let mut out = Connected::default();
    let results: Vec<Result<(Client, Vec<ToolInfo>), String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = servers
            .iter()
            .map(|server| {
                scope.spawn(move || {
                    let client = Client::connect(server, project, stop)?;
                    let tools = client.list_tools()?;
                    Ok((client, tools))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("the connection crashed".to_string()))).collect()
    });
    for (server, result) in servers.iter().zip(results) {
        let (client, tools) = match result {
            Ok(connected) => connected,
            Err(why) => {
                out.problems.push(format!("{}: couldn't connect — {}", server.name, one_line(&why, 160)));
                continue;
            }
        };
        let client = Arc::new(client);
        let mut joined = 0;
        for info in tools {
            if read_only && !info.read_only {
                out.left_out += 1;
                continue;
            }
            let exposed = exposed_name(&server.name, &info.name);
            if out.server_of.contains_key(&exposed) {
                out.problems.push(format!("{}: left out {} (another tool has the name {exposed})", server.name, info.name));
                continue;
            }
            out.server_of.insert(exposed.clone(), server.name.clone());
            out.tools.push(Box::new(McpTool { client: client.clone(), exposed, info }));
            joined += 1;
        }
        out.servers.push((server.name.clone(), joined));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    fn stdio(name: &str, command: &str, args: &[&str]) -> ServerConfig {
        let transport = Transport::Stdio { command: command.into(), args: args.iter().map(|a| a.to_string()).collect(), env: BTreeMap::new() };
        ServerConfig { name: name.into(), transport, timeout: None }
    }

    #[test]
    fn the_config_file_is_read_in_claudes_shape() {
        let servers = parse_config(
            r#"{ "mcpServers": {
                "files": { "command": "npx", "args": ["-y", "server-filesystem", "."], "env": { "KEY": "v" }, "timeout": 30 },
                "docs":  { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer x" } },
                "off":   { "command": "x", "disabled": true } } }"#,
        )
        .unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "docs");
        assert!(matches!(&servers[0].transport, Transport::Http { url, headers } if url == "https://example.com/mcp" && headers["Authorization"] == "Bearer x"));
        assert_eq!(servers[1].timeout, Some(30));
        assert!(matches!(&servers[1].transport, Transport::Stdio { command, args, env } if command == "npx" && args.len() == 3 && env["KEY"] == "v"));
    }

    #[test]
    fn a_broken_config_says_what_is_wrong() {
        assert!(parse_config("{ nope").unwrap_err().contains("not valid JSON"));
        assert!(parse_config("{}").unwrap_err().contains("mcpServers"));
        assert!(parse_config(r#"{ "mcpServers": { "x": {} } }"#).unwrap_err().contains("\"x\" needs"));
    }

    #[test]
    fn adding_and_removing_keeps_the_rest_of_the_file() {
        let dir = temp_dir("mcp-file");
        let file = dir.join("nested").join("mcp.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{ "theme": "dark", "mcpServers": { "old": { "command": "old-server" } } }"#).unwrap();

        assert!(!add_to_file(&file, &stdio("files", "npx", &["-y", "fs"])).unwrap());
        assert!(add_to_file(&file, &stdio("files", "npx", &["-y", "fs2"])).unwrap(), "same name: replaced");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"theme\": \"dark\""));
        let names: Vec<String> = parse_config(&text).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["files", "old"]);
        assert!(text.contains("fs2") && !text.contains("\"fs\""));

        assert!(remove_from_file(&file, "old").unwrap());
        assert!(!remove_from_file(&file, "old").unwrap());
        assert_eq!(parse_config(&std::fs::read_to_string(&file).unwrap()).unwrap().len(), 1);

        // A file that isn't JSON is left alone.
        std::fs::write(&file, "not json").unwrap();
        assert!(add_to_file(&file, &stdio("x", "y", &[])).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
        // No file yet: it is created, folder included.
        let fresh = dir.join("new").join("mcp.json");
        add_to_file(&fresh, &stdio("x", "y", &[])).unwrap();
        assert_eq!(parse_config(&std::fs::read_to_string(&fresh).unwrap()).unwrap()[0], stdio("x", "y", &[]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_projects_file_wins_over_the_users() {
        let dir = temp_dir("mcp-load");
        let (project, user) = (dir.join("p.json"), dir.join("u.json"));
        std::fs::write(&project, r#"{ "mcpServers": { "a": { "command": "project-a" } } }"#).unwrap();
        std::fs::write(&user, r#"{ "mcpServers": { "a": { "command": "user-a" }, "b": { "command": "user-b" } } }"#).unwrap();
        let (servers, problems) = load(&[project.clone(), dir.join("missing.json"), user]);
        assert!(problems.is_empty());
        assert_eq!(servers.len(), 2);
        assert!(matches!(&servers[0].0.transport, Transport::Stdio { command, .. } if command == "project-a"));
        assert_eq!(servers[0].1, project);
        std::fs::write(&project, "{ broken").unwrap();
        assert_eq!(load(&[project]).1.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn exposed_names_are_safe_for_model_servers() {
        assert_eq!(exposed_name("files", "read_file"), "files__read_file");
        assert_eq!(exposed_name("my server", "do.it/now"), "my_server__do_it_now");
        assert_eq!(exposed_name("s", &"t".repeat(100)).len(), 64);
    }

    /// A stand-in server written in sh: answers the handshake, lists two
    /// tools, and answers calls. Requests arrive in a known order, so it
    /// counts them instead of reading their ids.
    #[cfg(unix)]
    const FAKE_SERVER: &str = r#"
n=0
echo "starting up" >&2
echo "a log line that is not JSON"
while IFS= read -r line; do
  case "$line" in *'"result"'*) continue ;; *'"id"'*) n=$((n+1)) ;; *) continue ;; esac
  case "$line" in
    *'"initialize"'*) echo '{"jsonrpc":"2.0","id":'$n',"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}}}' ;;
    *'"tools/list"'*) echo '{"jsonrpc":"2.0","id":'$n',"result":{"tools":[{"name":"look","description":"Looks.","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","properties":{}}},{"name":"change","description":"Changes.","inputSchema":{"type":"object","properties":{"what":{"type":"string"}}}}]}}' ;;
    *'"name":"look"'*) echo '{"jsonrpc":"2.0","method":"notifications/progress","params":{}}'
                       echo '{"jsonrpc":"2.0","id":"srv-1","method":"roots/list"}'
                       echo '{"jsonrpc":"2.0","id":'$n',"result":{"content":[{"type":"text","text":"seen"}]}}' ;;
    *'"name":"change"'*) echo '{"jsonrpc":"2.0","id":'$n',"result":{"isError":true,"content":[{"type":"text","text":"refused by the server"}]}}' ;;
    *'"name":"hang"'*) sleep 30 ;;
    *'"name":"die"'*) echo "fatal: out of luck" >&2; exit 3 ;;
    *) echo '{"jsonrpc":"2.0","id":'$n',"error":{"code":-32601,"message":"no such method"}}' ;;
  esac
done
"#;

    #[cfg(unix)]
    fn fake(name: &str) -> ServerConfig {
        stdio(name, "sh", &["-c", FAKE_SERVER])
    }

    #[cfg(unix)]
    #[test]
    fn a_program_server_connects_lists_and_runs_tools() {
        let dir = temp_dir("mcp-stdio");
        let connected = connect_all(&[fake("fake")], &dir, false, &Stop::new());
        assert!(connected.problems.is_empty(), "{:?}", connected.problems);
        assert_eq!(connected.summary().unwrap(), "2 MCP tools from fake");
        let names: Vec<&str> = connected.tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["fake__look", "fake__change"]);
        assert_eq!(connected.server_of["fake__look"], "fake");
        // The read-only tool runs freely; the other asks.
        assert!(!connected.tools[0].needs_approval());
        assert!(connected.tools[1].needs_approval());

        let seen = connected.tools[0].run(&json!({}));
        assert!(seen.ok && seen.text == "seen", "{}", seen.text);
        let refused = connected.tools[1].run(&json!({ "what": "x" }));
        assert!(!refused.ok && refused.text == "refused by the server");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_mode_keeps_only_the_tools_that_change_nothing() {
        let dir = temp_dir("mcp-readonly");
        let connected = connect_all(&[fake("fake")], &dir, true, &Stop::new());
        assert_eq!(connected.tools.len(), 1);
        assert_eq!(connected.summary().unwrap(), "1 MCP tool from fake (read-only ones; 1 left out)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_server_that_hangs_dies_or_never_starts_is_reported_not_waited_for() {
        let dir = temp_dir("mcp-fail");
        let mut slow = fake("slow");
        slow.timeout = Some(1);
        let client = Client::connect(&slow, &dir, &Stop::new()).unwrap();
        let started = Instant::now();
        assert!(client.call_tool("hang", &json!({})).unwrap_err().contains("no answer within 1 s"));
        assert!(started.elapsed() < Duration::from_secs(5));

        // Ctrl-C ends the wait at once.
        let stop = Stop::new();
        let client = Client::connect(&fake("stoppable"), &dir, &stop).unwrap();
        stop.request();
        assert!(client.call_tool("hang", &json!({})).unwrap_err().contains("stopped"));

        let client = Client::connect(&fake("dying"), &dir, &Stop::new()).unwrap();
        let why = client.call_tool("die", &json!({})).unwrap_err();
        assert!(why.contains("stopped") && why.contains("out of luck"), "{why}");

        let connected = connect_all(&[stdio("ghost", "no-such-program-here", &[]), fake("ok")], &dir, false, &Stop::new());
        assert_eq!(connected.servers.len(), 1, "the working server still joins");
        assert!(connected.problems[0].starts_with("ghost: couldn't connect — couldn't start `no-such-program-here`"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stand-in web server for one exchange after another: `answers` are
    /// (content type, body) in the order the requests come.
    fn serve(answers: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (kind, body) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut seen = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buffer).unwrap();
                    seen.extend_from_slice(&buffer[..n]);
                    let text = String::from_utf8_lossy(&seen).to_string();
                    if let Some((head, sent)) = text.split_once("\r\n\r\n") {
                        let length: usize = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                        if sent.len() >= length || n == 0 {
                            requests.push(text);
                            break;
                        }
                    }
                }
                let status = if body.is_empty() { "202 Accepted" } else { "200 OK" };
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nMcp-Session-Id: s-42\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    #[test]
    fn a_web_server_connects_and_runs_a_tool_over_json_and_event_streams() {
        let dir = temp_dir("mcp-http");
        let (url, server) = serve(vec![
            ("application/json", json!({ "jsonrpc": "2.0", "id": 1, "result": { "protocolVersion": PROTOCOL } }).to_string()),
            ("application/json", String::new()),
            ("application/json", json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": [{ "name": "lookup", "annotations": { "readOnlyHint": true } }] } }).to_string()),
            // An event stream: a notification first, then the reply.
            ("text/event-stream", format!("event: message\ndata: {}\n\nevent: message\ndata: {}\n\n",
                json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": {} }),
                json!({ "jsonrpc": "2.0", "id": 3, "result": { "content": [{ "type": "text", "text": "found" }] } }))),
        ]);
        let headers = BTreeMap::from([("Authorization".to_string(), "Bearer k".to_string())]);
        let config = ServerConfig { name: "web".into(), transport: Transport::Http { url, headers }, timeout: None };
        let connected = connect_all(&[config], &dir, false, &Stop::new());
        assert!(connected.problems.is_empty(), "{:?}", connected.problems);
        let result = connected.tools[0].run(&json!({ "q": "x" }));
        assert!(result.ok && result.text == "found", "{}", result.text);

        let requests = server.join().unwrap();
        let lower: Vec<String> = requests.iter().map(|r| r.to_lowercase()).collect();
        assert!(lower[0].contains("authorization: bearer k") && lower[0].contains("\"initialize\""));
        assert!(!lower[0].contains("mcp-session-id"), "no session before the server gives one");
        assert!(lower[2].contains("mcp-session-id: s-42") && lower[2].contains("mcp-protocol-version"));
        assert!(lower[3].contains("\"tools/call\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
