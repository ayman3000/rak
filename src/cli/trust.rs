//! Folder trust, from maki: what a project folder brings of its own is used
//! only once the person has said they trust that folder. A cloned repository
//! can ship a `.rak/mcp.json` (MCP servers are programs rak starts), a `.env`
//! that points rak's model requests, and the provider key with them, at
//! another address (`RAK_BASE_URL`), and skills and routines the agent
//! follows. Opening a repository must not run or follow any of that unasked.
//!
//! Asked once per folder, in a terminal; the answer "yes" is remembered in
//! ~/.config/rak/trusted (one folder per line). Without a terminal (a script,
//! CI, an unattended -y run) a folder is trusted only if it already was, or
//! with --trust for that run. The project's instructions (AGENTS.md,
//! rak/rules.md) are read as before: every agent reads them, and they ask
//! nothing of rak itself.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Pure — unit-tested. A .env name rak reads: its own settings, a provider's
/// key, the web search key.
pub fn rak_setting(name: &str) -> bool {
    name.starts_with("RAK_") || name.ends_with("_API_KEY") || name == "HF_TOKEN"
}

/// What a project folder brings of its own, one line each, for the question.
/// Empty: nothing to trust.
pub fn project_settings(project: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let rak = project.join(".rak");
    if let Ok(text) = std::fs::read_to_string(rak.join("mcp.json")) {
        let names: Vec<String> =
            rak::mcp::parse_config(&text).map(|servers| servers.into_iter().map(|s| s.name).collect()).unwrap_or_default();
        out.push(if names.is_empty() {
            ".rak/mcp.json (MCP servers, which are programs rak starts)".to_string()
        } else {
            format!(".rak/mcp.json: MCP servers {} (programs rak starts)", names.join(", "))
        });
    }
    for (folder, what) in [("skills", "skills the agent follows"), ("routines", "routines the agent can run")] {
        let count = std::fs::read_dir(rak.join(folder)).map(|entries| entries.count()).unwrap_or(0);
        if count > 0 {
            out.push(format!(".rak/{folder}: {count} {what}"));
        }
    }
    if let Ok(text) = std::fs::read_to_string(project.join(".env")) {
        let mut names: Vec<String> = super::settings::parse_env(&text).into_keys().filter(|n| rak_setting(n)).collect();
        names.sort();
        if !names.is_empty() {
            out.push(format!(".env: {} (where rak sends requests, and with which key)", names.join(", ")));
        }
    }
    out
}

fn store() -> Option<PathBuf> {
    super::settings::config_dir().map(|dir| dir.join("trusted"))
}

fn canonical(project: &Path) -> String {
    std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf()).to_string_lossy().to_string()
}

/// Pure — unit-tested. Whether `folder` is a line of the trusted list.
pub fn listed(list: &str, folder: &str) -> bool {
    list.lines().any(|line| line.trim() == folder)
}

pub fn is_trusted(project: &Path) -> bool {
    store().and_then(|file| std::fs::read_to_string(file).ok()).is_some_and(|list| listed(&list, &canonical(project)))
}

/// Adds the folder to the trusted list (rak itself wrote into its .rak/, or
/// the person said yes).
pub fn remember(project: &Path) {
    let Some(file) = store() else { return };
    let folder = canonical(project);
    let list = std::fs::read_to_string(&file).unwrap_or_default();
    if listed(&list, &folder) {
        return;
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, format!("{list}{folder}\n"));
}

/// Whether to use what the project folder brings, and a note to show when
/// it is left out. Asks in a terminal when the folder is new.
pub fn decide(project: &Path, forced: bool, can_ask: bool, color: bool) -> (bool, Option<String>) {
    let brings = project_settings(project);
    if brings.is_empty() || forced || is_trusted(project) {
        return (true, None);
    }
    if can_ask {
        let question = format!(
            "This folder has rak settings of its own:\n{}\nTrust this folder and use them? [y/N] › ",
            brings.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n")
        );
        eprint!("{}", super::style::paint(color, super::style::Style::Yellow, &question));
        let _ = std::io::stderr().flush();
        let mut typed = String::new();
        let _ = std::io::stdin().lock().read_line(&mut typed);
        if super::chain::is_yes(&typed) {
            remember(project);
            return (true, None);
        }
    }
    let note = format!(
        "Not using this folder's own rak settings ({}): it is not trusted. Start rak here in a terminal to be asked, \
         or pass --trust.",
        brings.iter().map(|l| l.split([':', ' ']).next().unwrap_or(l).to_string()).collect::<Vec<_>>().join(", ")
    );
    (false, Some(note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rak-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_settings_rak_reads_make_a_folder_need_trust() {
        assert!(rak_setting("RAK_BASE_URL") && rak_setting("ANTHROPIC_API_KEY") && rak_setting("HF_TOKEN"));
        assert!(!rak_setting("DATABASE_URL") && !rak_setting("PORT"));
        let dir = temp_dir("trust");
        std::fs::write(dir.join(".env"), "DATABASE_URL=postgres://x\n").unwrap();
        assert!(project_settings(&dir).is_empty(), "an app's own .env asks nothing");
        std::fs::write(dir.join(".env"), "DATABASE_URL=x\nRAK_BASE_URL=https://elsewhere\n").unwrap();
        std::fs::create_dir_all(dir.join(".rak/skills")).unwrap();
        std::fs::write(dir.join(".rak/skills/deploy.md"), "# Deploy\n").unwrap();
        std::fs::write(dir.join(".rak/mcp.json"), r#"{"mcpServers":{"evil":{"command":"sh","args":["-c","x"]}}}"#).unwrap();
        let lines = project_settings(&dir);
        assert_eq!(lines.len(), 3, "{lines:#?}");
        assert!(lines[0].contains("MCP servers evil"));
        assert!(lines[1].starts_with(".rak/skills: 1"));
        assert!(lines[2].starts_with(".env: RAK_BASE_URL"));
    }

    #[test]
    fn an_untrusted_folder_without_a_terminal_is_left_out_and_said_so() {
        let dir = temp_dir("trust-no-tty");
        std::fs::create_dir_all(dir.join(".rak")).unwrap();
        std::fs::write(dir.join(".rak/mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        let (trusted, note) = decide(&dir, false, false, false);
        assert!(!trusted && note.unwrap().contains("--trust"));
        assert!(decide(&dir, true, false, false).0, "--trust");
        assert_eq!(decide(&temp_dir("trust-empty"), false, false, false), (true, None), "nothing to trust");
    }

    #[test]
    fn the_list_matches_whole_lines() {
        assert!(listed("/a/b\n/c\n", "/c"));
        assert!(!listed("/a/bc\n", "/a/b"));
    }
}
