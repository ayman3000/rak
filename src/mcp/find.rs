//! Finding MCP servers in text a person points rak at: the JSON a server's
//! page tells you to paste into "your MCP client", in the shapes those pages
//! use, wherever it sits in a README or a web page. Nothing here asks or
//! saves; `rak mcp add` shows what was found and asks first.

use super::config::{ServerConfig, Transport};
use serde_json::Value;
use std::collections::BTreeMap;

/// The most text one JSON object in a page may take.
const MAX_OBJECT: usize = 20_000;

/// Pure — unit-tested. Every distinct server described in `text`, in the
/// order found. One with no name of its own (a bare `{ "command": … }`)
/// comes back with an empty name. Entries for Windows' command shell are
/// left out elsewhere when the page has others.
pub fn servers_in(text: &str, windows: bool) -> Vec<ServerConfig> {
    let text = if looks_like_html(text) { page_text(text) } else { text.to_string() };
    let mut found: Vec<ServerConfig> = Vec::new();
    for object in objects(&text) {
        for server in servers_of(&object) {
            // A page repeats its config for each client, often with only the
            // name's spelling or npx's -y changed: one of each is offered,
            // the first with a name.
            match found.iter_mut().find(|have| same_thing(have, &server)) {
                Some(have) if have.name.is_empty() => have.name = server.name,
                Some(_) => {}
                None => found.push(server),
            }
        }
    }
    let for_windows = |s: &ServerConfig| matches!(&s.transport, Transport::Stdio { command, .. } if ["cmd", "cmd.exe", "powershell", "powershell.exe", "pwsh"].contains(&command.to_lowercase().as_str()));
    if !windows && found.iter().any(|s| !for_windows(s)) {
        found.retain(|s| !for_windows(s));
    }
    found
}

/// The same program with the same variables, or the same address with the
/// same headers, whatever the name; `npx X` and `npx -y X` are one.
fn same_thing(a: &ServerConfig, b: &ServerConfig) -> bool {
    let plain = |args: &[String]| -> Vec<String> { args.iter().filter(|arg| *arg != "-y" && *arg != "--yes").cloned().collect() };
    match (&a.transport, &b.transport) {
        (Transport::Stdio { command: c1, args: a1, env: e1 }, Transport::Stdio { command: c2, args: a2, env: e2 }) => {
            c1 == c2 && plain(a1) == plain(a2) && e1.keys().eq(e2.keys())
        }
        (Transport::Http { url: u1, headers: h1 }, Transport::Http { url: u2, headers: h2 }) => {
            u1.trim_end_matches('/') == u2.trim_end_matches('/') && h1.keys().eq(h2.keys())
        }
        _ => false,
    }
}

fn looks_like_html(text: &str) -> bool {
    let start: String = text.chars().take(2000).collect::<String>().to_lowercase();
    start.contains("<!doctype html") || start.contains("<html")
}

/// Pure — unit-tested. A web page as plain text: tags dropped and the
/// entities a code block is written with turned back into characters.
pub fn page_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x22;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

/// Pure — unit-tested. The outermost JSON objects written in `text`: a
/// README's code blocks, or the text itself. Braces that are not JSON
/// (code, prose) are passed over.
pub fn objects(text: &str) -> Vec<Value> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'{' {
            at += 1;
            continue;
        }
        match closing(bytes, at).and_then(|end| lenient(&text[at..=end]).map(|value| (end, value))) {
            Some((end, value)) if value.is_object() => {
                out.push(value);
                at = end + 1;
            }
            _ => at += 1,
        }
    }
    out
}

/// JSON as pages write it: strict first, then with `//` comments and commas
/// before a closing bracket taken out, which READMEs often leave in.
fn lenient(text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut in_string, mut escaped, mut i) = (String::with_capacity(text.len()), false, false, 0);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == ',' && chars[i + 1..].iter().find(|c| !c.is_whitespace()).is_some_and(|next| *next == '}' || *next == ']') {
            // dropped
        } else {
            out.push(c);
        }
        i += 1;
    }
    serde_json::from_str::<Value>(&out).ok()
}

/// Where the brace opened at `from` closes, strings respected.
fn closing(bytes: &[u8], from: usize) -> Option<usize> {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (i, &b) in bytes.iter().enumerate().skip(from).take(MAX_OBJECT) {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Pure — unit-tested. The servers one JSON object describes, in the shapes
/// MCP pages use: `mcpServers` (Claude, Cursor), `servers` (VS Code), either
/// under `mcp`, a bare map of names to entries, or one entry alone.
pub fn servers_of(value: &Value) -> Vec<ServerConfig> {
    let Some(object) = value.as_object() else { return Vec::new() };
    for key in ["mcpServers", "servers", "mcp_servers", "context_servers"] {
        if let Some(map) = object.get(key).and_then(Value::as_object) {
            return map.iter().filter_map(|(name, entry)| server(name, entry)).collect();
        }
    }
    if let Some(inner) = object.get("mcp").filter(|inner| inner.is_object()) {
        return servers_of(inner);
    }
    // Outside those keys an object must look like a server to count as one:
    // a tool call's example arguments have a "url" too.
    if let Some(alone) = server("", value).filter(|_| plainly_a_server(value)) {
        return vec![alone];
    }
    object.iter().filter(|(_, entry)| plainly_a_server(entry)).filter_map(|(name, entry)| server(name, entry)).collect()
}

/// A program to run, or a web address said to be an MCP one (a transport
/// type, or headers to send).
fn plainly_a_server(entry: &Value) -> bool {
    let kind = entry["type"].as_str().or_else(|| entry["transport"].as_str()).unwrap_or("").to_lowercase();
    // One word: a whole command line ("grep -r x /lib") is a shell tool's example argument.
    let program = match entry.get("command") {
        Some(Value::String(word)) => !word.trim().is_empty() && !word.trim().contains(char::is_whitespace),
        Some(Value::Array(list)) => !list.is_empty(),
        _ => false,
    };
    program
        || ["http", "sse", "streamable-http", "streamablehttp", "streamable_http", "remote"].contains(&kind.as_str())
        || entry.get("headers").is_some_and(Value::is_object)
}

fn strings(value: Option<&Value>) -> BTreeMap<String, String> {
    value.and_then(Value::as_object).into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()
}

fn server(name: &str, entry: &Value) -> Option<ServerConfig> {
    let entry_object = entry.as_object()?;
    let words = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect() };
    // "command" is a word, or (opencode) the program and its arguments together.
    let mut command: Vec<String> = match entry_object.get("command") {
        Some(Value::String(word)) if !word.trim().is_empty() => vec![word.trim().to_string()],
        Some(list @ Value::Array(_)) => words(list),
        _ => Vec::new(),
    };
    command.extend(entry_object.get("args").map(words).unwrap_or_default());
    let url = ["url", "endpoint", "serverUrl", "httpUrl"]
        .iter()
        .find_map(|key| entry_object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"));
    let transport = match (command.split_first(), url) {
        (Some((program, args)), _) => Transport::Stdio {
            command: program.clone(),
            args: args.to_vec(),
            env: strings(entry_object.get("env").or_else(|| entry_object.get("environment"))),
        },
        (None, Some(url)) => Transport::Http { url: url.to_string(), headers: strings(entry_object.get("headers")) },
        (None, None) => return None,
    };
    Some(ServerConfig { name: name.trim().to_string(), transport, timeout: entry_object.get("timeout").and_then(Value::as_u64) })
}

/// A value a page left for the reader to fill in.
#[derive(Debug, Clone, PartialEq)]
pub enum Blank {
    Env(String),
    Header(String),
    /// The argument at this place (0: the first after the program).
    Arg(usize),
}

/// Pure — unit-tested. Whether a value is a stand-in ("<YOUR_API_KEY>",
/// "/path/to/project", "your-token-here") and not something to run as it is.
pub fn is_stand_in(value: &str) -> bool {
    let low = value.to_lowercase();
    value.trim().is_empty()
        || (value.contains('<') && value.contains('>'))
        || ["your_", "your-", "your ", "yourkey", "xxxx", "****", "...", "${", "{{", "/path/to", "path/to/", "changeme", "_here", "-here"]
            .iter()
            .any(|sign| low.contains(sign))
}

/// Pure — unit-tested. What the person must fill in before this server can
/// be saved: stand-in values among its variables, headers and arguments.
pub fn blanks(server: &ServerConfig) -> Vec<Blank> {
    match &server.transport {
        Transport::Stdio { args, env, .. } => args
            .iter()
            .enumerate()
            .filter(|(_, arg)| is_stand_in(arg))
            .map(|(i, _)| Blank::Arg(i))
            .chain(env.iter().filter(|(_, v)| is_stand_in(v)).map(|(k, _)| Blank::Env(k.clone())))
            .collect(),
        Transport::Http { headers, .. } => headers.iter().filter(|(_, v)| is_stand_in(v)).map(|(k, _)| Blank::Header(k.clone())).collect(),
    }
}

/// Pure — unit-tested. Puts the person's value where the blank was.
pub fn fill(server: &mut ServerConfig, blank: &Blank, value: &str) {
    match (&mut server.transport, blank) {
        (Transport::Stdio { args, .. }, Blank::Arg(i)) if *i < args.len() => args[*i] = value.to_string(),
        (Transport::Stdio { env, .. }, Blank::Env(key)) => {
            env.insert(key.clone(), value.to_string());
        }
        (Transport::Http { headers, .. }, Blank::Header(key)) => {
            headers.insert(key.clone(), value.to_string());
        }
        _ => {}
    }
}

/// Pure — unit-tested. What a server is, for the person to read before
/// saying yes: what runs or where it connects, and the names (never the
/// values) of what is sent with it.
pub fn describe(server: &ServerConfig) -> Vec<String> {
    match &server.transport {
        Transport::Stdio { command, args, env } => {
            let mut lines = vec![format!("runs:  {}", std::iter::once(command).chain(args).cloned().collect::<Vec<_>>().join(" "))];
            if !env.is_empty() {
                lines.push(format!("with:  {}", env.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            lines
        }
        Transport::Http { url, headers } => {
            let mut lines = vec![format!("connects to:  {url}")];
            if !headers.is_empty() {
                lines.push(format!("sends:  {}", headers.keys().cloned().collect::<Vec<_>>().join(", ")));
            }
            lines
        }
    }
}

/// Pure — unit-tested. What the person should know before saying yes.
pub fn caution(server: &ServerConfig) -> String {
    match &server.transport {
        Transport::Stdio { command, args, .. } => {
            let program = command.rsplit(['/', '\\']).next().unwrap_or(command).to_lowercase();
            let fetches = ["npx", "bunx", "pnpx", "uvx", "pipx"].contains(&program.as_str())
                || (["pnpm", "yarn"].contains(&program.as_str()) && args.first().is_some_and(|a| a == "dlx"));
            if fetches {
                "This is a program rak will start on your machine. It is downloaded when it starts, so what runs is whatever its publisher last released.".to_string()
            } else if program == "docker" || program == "podman" {
                "This is a container rak will start on your machine.".to_string()
            } else {
                "This is a program rak will start on your machine.".to_string()
            }
        }
        Transport::Http { .. } => "This is a server on the web: what you ask its tools to do is sent to that address.".to_string(),
    }
}

/// Pure — unit-tested. A server name from what the person typed or where the
/// JSON came from: letters, digits, - and _ only.
pub fn tidy_name(raw: &str) -> String {
    let last = raw.trim().trim_end_matches('/').rsplit('/').next().unwrap_or(raw);
    let last = last.strip_suffix(".git").unwrap_or(last);
    let name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    name.trim_matches('-').to_string()
}

/// Pure — unit-tested. Where to read a page's text from: a GitHub
/// repository's README (two spellings), or the address itself with a GitHub
/// file page turned into its raw file.
pub fn places_to_read(address: &str) -> Vec<String> {
    match crate::skills::fetch::github_repo(address) {
        Some((owner, repo)) => ["README.md", "readme.md", "Readme.md"]
            .iter()
            .map(|file| format!("https://raw.githubusercontent.com/{owner}/{repo}/HEAD/{file}"))
            .collect(),
        None => vec![crate::skills::fetch::raw_url(address)],
    }
}

/// Pure — unit-tested. A command line as its words: spaces separate, and
/// single or double quotes keep a word together.
pub fn words(line: &str) -> Vec<String> {
    let (mut out, mut word, mut quote, mut started) = (Vec::new(), String::new(), None::<char>, false);
    for c in line.trim().chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => word.push(c),
        }
    }
    if started || !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stdio(name: &str, command: &str, args: &[&str]) -> ServerConfig {
        let transport =
            Transport::Stdio { command: command.into(), args: args.iter().map(|a| a.to_string()).collect(), env: BTreeMap::new() };
        ServerConfig { name: name.into(), transport, timeout: None }
    }

    const README: &str = r#"# A server

Some prose with {braces} that are not JSON, and code: `fn main() { }`.

Add the following config to your MCP client:

```json
{
  "mcpServers": {
    "chrome-devtools": {
      "command": "npx",
      "args": ["-y", "chrome-devtools-mcp@latest"]
    }
  }
}
```

The same again for another client:

```json
{ "mcpServers": { "chrome-devtools": { "command": "npx", "args": ["-y", "chrome-devtools-mcp@latest"] } } }
```

On Windows:

```json
{ "mcpServers": { "chrome-devtools": { "command": "cmd", "args": ["/c", "npx", "-y", "chrome-devtools-mcp@latest"] } } }
```
"#;

    #[test]
    fn a_readmes_config_is_found_once_and_the_windows_one_is_left_to_windows() {
        assert_eq!(servers_in(README, false), [stdio("chrome-devtools", "npx", &["-y", "chrome-devtools-mcp@latest"])]);
        let on_windows = servers_in(README, true);
        assert_eq!(on_windows.len(), 2, "both are offered there");
        // A page with only the Windows form still offers it.
        let only = r#"{ "mcpServers": { "x": { "command": "cmd", "args": ["/c", "npx", "x"] } } }"#;
        assert_eq!(servers_in(only, false).len(), 1);
        assert!(servers_in("no json here { at all }", false).is_empty());
    }

    #[test]
    fn the_shapes_mcp_pages_use_are_all_read() {
        let one = |v: Value| servers_of(&v);
        let expected = stdio("files", "npx", &["-y", "fs", "."]);
        assert_eq!(
            one(json!({ "mcpServers": { "files": { "command": "npx", "args": ["-y", "fs", "."] } } })),
            std::slice::from_ref(&expected)
        );
        assert_eq!(
            one(json!({ "servers": { "files": { "type": "stdio", "command": "npx", "args": ["-y", "fs", "."] } } })),
            std::slice::from_ref(&expected)
        );
        assert_eq!(
            one(json!({ "mcp": { "servers": { "files": { "command": "npx", "args": ["-y", "fs", "."] } } } })),
            std::slice::from_ref(&expected)
        );
        assert_eq!(one(json!({ "files": { "command": "npx", "args": ["-y", "fs", "."] } })), std::slice::from_ref(&expected), "a bare map");
        // The program and its arguments together, and "environment".
        let together =
            one(json!({ "mcp": { "files": { "type": "local", "command": ["npx", "-y", "fs", "."], "environment": { "K": "v" } } } }));
        assert!(
            matches!(&together[0].transport, Transport::Stdio { command, args, env } if command == "npx" && args.len() == 3 && env["K"] == "v")
        );
        // One entry alone has no name yet.
        assert_eq!(one(json!({ "command": "npx", "args": ["-y", "fs", "."] })), [stdio("", "npx", &["-y", "fs", "."])]);
        let web = one(
            json!({ "mcpServers": { "docs": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer <TOKEN>" } } } }),
        );
        assert!(matches!(&web[0].transport, Transport::Http { url, headers } if url == "https://example.com/mcp" && headers.len() == 1));
        assert!(one(json!({ "name": "pkg", "version": "1.0.0" })).is_empty(), "a package.json is not a server");
        assert!(one(json!({ "mcpServers": { "x": { "url": "not-an-address" } } })).is_empty());
    }

    #[test]
    fn json_is_found_in_a_web_page_and_among_other_braces() {
        let html = "<!DOCTYPE html><html><body><pre><code>{\n  <span>&quot;mcpServers&quot;</span>: { &quot;a&quot;: { &quot;command&quot;: &quot;a-server&quot;, &quot;args&quot;: [&quot;--x&quot;] } }\n}</code></pre></body></html>";
        assert_eq!(servers_in(html, false), [stdio("a", "a-server", &["--x"])]);
        // A brace inside a string does not end the object early.
        let tricky = r#"text {"mcpServers":{"a":{"command":"sh","args":["-c","echo } {"]}}} more"#;
        assert_eq!(servers_in(tricky, false), [stdio("a", "sh", &["-c", "echo } {"])]);
        assert_eq!(objects("{ not json } {\"a\": 1} {\"b\": {\"c\": 2}}").len(), 2);
    }

    #[test]
    fn what_real_pages_do_is_handled() {
        // A comma left before a closing brace, and a comment (exa's README has the first).
        let loose = "{\n  \"mcpServers\": {\n    // the hosted one\n    \"exa\": { \"type\": \"streamable-http\", \"url\": \"https://mcp.exa.ai/mcp\", }\n  }\n}";
        let found = servers_in(loose, false);
        assert!(matches!(&found[0].transport, Transport::Http { url, .. } if url == "https://mcp.exa.ai/mcp"), "{found:?}");
        // A "//" inside a string is not a comment.
        assert_eq!(servers_in(r#"{ "mcpServers": { "a": { "command": "a", "args": ["http://x//y"], } } }"#, false).len(), 1);

        // The same server repeated for each client, with -y added, the name's
        // spelling changed or no name at all, is offered once, with a name.
        let repeated = r#"
            { "command": "npx", "args": ["@playwright/mcp@latest"] }
            { "mcpServers": { "playwright": { "command": "npx", "args": ["-y", "@playwright/mcp@latest"] } } }
            { "servers": { "Playwright": { "command": "npx", "args": ["@playwright/mcp@latest"] } } }
            { "mcpServers": { "playwright": { "command": "docker", "args": ["run", "-i", "mcp"] } } }"#;
        let found = servers_in(repeated, false);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].name, "playwright");
        // Different variables make a different choice.
        let variants = r#"{ "mcpServers": { "s": { "command": "s", "env": { "A": "1" } } } }
                          { "mcpServers": { "s": { "command": "s", "env": { "B": "1" } } } }"#;
        assert_eq!(servers_in(variants, false).len(), 2);

        // A tool call's example is not a server (firecrawl's README has one).
        assert!(servers_in(r#"{ "name": "scrape", "arguments": { "url": "https://example.com/product" } }"#, false).is_empty());
        assert!(servers_in(r#"{ "url": "https://example.com/api" }"#, false).is_empty());
        assert!(servers_in(r#"{ "name": "shell", "arguments": { "command": "grep -ril podcast /library" } }"#, false).is_empty());
        assert!(servers_in(r#"{ "command": "cat /library/README.md" }"#, false).is_empty());
        // A web address said to be an MCP one, outside "mcpServers", still counts.
        assert_eq!(servers_in(r#"{ "type": "http", "url": "https://example.com/mcp" }"#, false).len(), 1);
        assert_eq!(servers_in(r#"{ "docs": { "url": "https://example.com/mcp", "headers": { "X": "y" } } }"#, false).len(), 1);
        assert!(is_stand_in("NOTION_TOKEN=ntn_****"));
    }

    #[test]
    fn stand_ins_are_found_and_filled() {
        for value in ["<YOUR_API_KEY>", "your-token-here", "YOUR_KEY", "/path/to/project", "sk-xxxx", "", "${API_KEY}", "..."] {
            assert!(is_stand_in(value), "{value}");
        }
        for value in ["true", "-y", "chrome-devtools-mcp@latest", ".", "/Users/me/code", "--headless"] {
            assert!(!is_stand_in(value), "{value}");
        }
        let mut server = servers_of(&json!({ "mcpServers": { "gh": {
            "command": "npx", "args": ["-y", "gh-mcp", "/path/to/repo"], "env": { "TOKEN": "<YOUR_TOKEN>", "MODE": "fast" } } } }))
        .remove(0);
        assert_eq!(blanks(&server), [Blank::Arg(2), Blank::Env("TOKEN".into())]);
        fill(&mut server, &Blank::Arg(2), "/code");
        fill(&mut server, &Blank::Env("TOKEN".into()), "t-1");
        assert!(blanks(&server).is_empty());
        assert!(
            matches!(&server.transport, Transport::Stdio { args, env, .. } if args[2] == "/code" && env["TOKEN"] == "t-1" && env["MODE"] == "fast")
        );
        // What is shown names the variables and never their values.
        let shown = describe(&server).join("\n");
        assert!(shown.contains("runs:  npx -y gh-mcp /code") && shown.contains("MODE, TOKEN") && !shown.contains("t-1"), "{shown}");
    }

    #[test]
    fn the_caution_says_what_kind_of_thing_runs() {
        assert!(caution(&stdio("a", "npx", &["-y", "x@latest"])).contains("downloaded when it starts"));
        assert!(caution(&stdio("a", "uvx", &["x"])).contains("downloaded when it starts"));
        assert!(caution(&stdio("a", "docker", &["run", "x"])).contains("container"));
        let plain = caution(&stdio("a", "/usr/local/bin/my-server", &[]));
        assert!(plain.contains("program rak will start") && !plain.contains("downloaded"));
        let web = ServerConfig {
            name: "d".into(),
            transport: Transport::Http { url: "https://e.com/mcp".into(), headers: BTreeMap::new() },
            timeout: None,
        };
        assert!(caution(&web).contains("sent to that address"));
    }

    #[test]
    fn names_places_and_command_lines_are_read() {
        assert_eq!(tidy_name("https://github.com/ChromeDevTools/chrome-devtools-mcp"), "chrome-devtools-mcp");
        assert_eq!(tidy_name("https://github.com/o/my.server.git/"), "my-server");
        assert_eq!(tidy_name("  docs  "), "docs");
        let places = places_to_read("https://github.com/o/r");
        assert_eq!(places[0], "https://raw.githubusercontent.com/o/r/HEAD/README.md");
        assert_eq!(places.len(), 3);
        assert_eq!(places_to_read("https://example.com/docs/mcp"), ["https://example.com/docs/mcp"]);
        assert_eq!(words("npx -y some-mcp ."), ["npx", "-y", "some-mcp", "."]);
        assert_eq!(words("  sh -c 'echo hi there' \"a b\" ''  "), ["sh", "-c", "echo hi there", "a b", ""]);
        assert!(words("   ").is_empty());
    }
}
