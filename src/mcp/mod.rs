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
//! server adds its tool descriptions to every request — a cost worth knowing —
//! except a large one, whose tools are sent only after the model loads them
//! with `load_tools` (see `Connected::to_defer`).
//!
//!   config   the mcp.json files: read, added to, removed from
//!   client   one connection: the handshake, tools/list, tools/call
//!   (here)   a server's tools as the agent's, and connecting them all

mod client;
mod config;

#[cfg(unix)]
pub use client::stop_all_servers_now;
pub use client::{result_text, tool_info, Client, ToolInfo};
pub use config::{add_to_file, config_files, entry_json, load, parse_config, remove_from_file, ServerConfig, Transport};

use crate::agent::ToolGroup;
use crate::interrupt::Stop;
use crate::text::one_line;
use crate::tools::{Tool, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

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
    let clean =
        |text: &str| -> String { text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect() };
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
    /// Each connected server's tools as a group, with the size of their
    /// definitions in characters (what they add to every request).
    pub groups: Vec<(ToolGroup, usize)>,
}

/// A server whose tool definitions are longer than this (about 1,000 tokens)
/// is deferred: its tools are sent only after the model loads them.
pub const DEFER_OVER_CHARS: usize = 3_000;

impl Connected {
    /// Pure — unit-tested. The groups to hide until loaded: the large ones.
    pub fn to_defer(&self) -> Vec<ToolGroup> {
        self.groups.iter().filter(|(_, size)| *size > DEFER_OVER_CHARS).map(|(g, _)| g.clone()).collect()
    }
}

/// Pure — unit-tested. A group's one line: the server's own first sentence
/// when it gives one, else its tools' names.
pub fn group_description(instructions: Option<&str>, tool_names: &[String]) -> String {
    if let Some(first) = instructions.map(|t| one_line(t.split(". ").next().unwrap_or(t), 140)).filter(|t| !t.is_empty()) {
        return first.trim_end_matches('.').to_string();
    }
    let shown: Vec<&str> = tool_names.iter().take(8).map(String::as_str).collect();
    let more = if tool_names.len() > shown.len() { ", …" } else { "" };
    format!("tools {}{more}", shown.join(", "))
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
        let deferred: Vec<String> = self.to_defer().into_iter().map(|g| g.id).collect();
        let later = if deferred.is_empty() { String::new() } else { format!("; {} loaded when needed", deferred.join(", ")) };
        Some(format!("{count} MCP tool{} from {}{note}{later}", if count == 1 { "" } else { "s" }, names.join(", ")))
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
        let (mut exposed_names, mut plain_names, mut size) = (Vec::new(), Vec::new(), 0);
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
            exposed_names.push(exposed.clone());
            plain_names.push(info.name.clone());
            let tool: Box<dyn Tool> = Box::new(McpTool { client: client.clone(), exposed, info });
            size += crate::tools::definition(tool.as_ref()).to_string().len();
            out.tools.push(tool);
            joined += 1;
        }
        if joined > 0 {
            let description = group_description(client.instructions.as_deref(), &plain_names);
            out.groups.push((ToolGroup { id: server.name.clone(), description, tools: exposed_names }, size));
        }
        out.servers.push((server.name.clone(), joined));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::client::PROTOCOL;
    use super::*;
    use crate::tools::testing::temp_dir;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::time::{Duration, Instant};

    fn stdio(name: &str, command: &str, args: &[&str]) -> ServerConfig {
        let transport =
            Transport::Stdio { command: command.into(), args: args.iter().map(|a| a.to_string()).collect(), env: BTreeMap::new() };
        ServerConfig { name: name.into(), transport, timeout: None }
    }

    #[test]
    fn large_servers_are_deferred_with_a_one_line_description() {
        let names = |n: usize| (0..n).map(|i| format!("t{i}")).collect::<Vec<_>>();
        assert_eq!(group_description(Some("Read and write GitHub issues. Needs a token."), &names(3)), "Read and write GitHub issues");
        assert_eq!(group_description(None, &names(3)), "tools t0, t1, t2");
        assert_eq!(group_description(Some("  "), &names(9)), "tools t0, t1, t2, t3, t4, t5, t6, t7, …");
        let group = |id: &str| ToolGroup { id: id.into(), description: String::new(), tools: vec![] };
        let connected = Connected { groups: vec![(group("small"), 900), (group("big"), DEFER_OVER_CHARS + 1)], ..Default::default() };
        assert_eq!(connected.to_defer(), [group("big")]);
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
        let (group, size) = &connected.groups[0];
        assert_eq!((group.id.as_str(), group.tools.clone()), ("fake", vec!["fake__look".to_string(), "fake__change".to_string()]));
        assert!(*size > 100 && connected.to_defer().is_empty(), "two small tools stay loaded");
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
                        let length: usize = head
                            .lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                            .unwrap_or(0);
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
