//! The servers' list: `mcp.json`, read, and edited by `rak mcp add|remove`.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    fn stdio(name: &str, command: &str, args: &[&str]) -> ServerConfig {
        let transport =
            Transport::Stdio { command: command.into(), args: args.iter().map(|a| a.to_string()).collect(), env: BTreeMap::new() };
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
        assert!(
            matches!(&servers[0].transport, Transport::Http { url, headers } if url == "https://example.com/mcp" && headers["Authorization"] == "Bearer x")
        );
        assert_eq!(servers[1].timeout, Some(30));
        assert!(
            matches!(&servers[1].transport, Transport::Stdio { command, args, env } if command == "npx" && args.len() == 3 && env["KEY"] == "v")
        );
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
}
