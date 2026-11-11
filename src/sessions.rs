//! Saved conversations. Each chat is a JSON file in a folder per project, so
//! `rak -c` picks up this project's last one and `/resume` lists them. Learned
//! from sak's `-c` and `/resume`, with two changes: the chat is saved after
//! every request (nothing is lost when the terminal closes), and every chat
//! keeps its own file (starting a new one does not overwrite the last).
//!
//! The system prompt is not saved: a resumed chat gets today's, with today's
//! skills and notes.

use crate::text::one_line;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Chats kept per project; older ones are deleted when a new one is saved.
pub const KEEP: usize = 30;

/// A saved chat, as `list` describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub path: PathBuf,
    /// The first request, on one line.
    pub title: String,
    /// Seconds since 1970.
    pub saved_at: u64,
    /// Messages in it, the system prompt not counted.
    pub messages: usize,
}

/// Pure — unit-tested. The folder name for a project's chats: its own name
/// plus a hash of its whole path, so two projects called `app` stay apart.
pub fn project_key(project: &Path) -> String {
    let path = project.to_string_lossy();
    let hash = path.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let name: String = project
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    format!("{}-{hash:08x}", if name.is_empty() { "root" } else { &name })
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A new chat's id: sortable by time.
pub fn new_id() -> String {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}-{:03}", at.as_secs(), at.subsec_millis())
}

/// Pure — unit-tested. A chat's title: its first request, on one line.
pub fn title(history: &[Value]) -> String {
    history
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .map(|text| one_line(text, 70))
        .unwrap_or_else(|| "(empty)".to_string())
}

/// Saves `history` (the conversation without its system prompt) as chat `id`
/// of the project, and deletes the project's oldest chats past `KEEP`.
pub fn save(dir: &Path, key: &str, id: &str, history: &[Value], model: &str) -> Result<PathBuf, String> {
    let folder = dir.join(key);
    std::fs::create_dir_all(&folder).map_err(|e| format!("cannot make {}: {e}", folder.display()))?;
    let path = folder.join(format!("{id}.json"));
    let body = json!({ "version": 1, "saved_at": now(), "model": model, "title": title(history), "messages": history });
    // Written beside it, then moved over it: a crash mid-write leaves the old file whole.
    let partial = folder.join(format!(".{id}.json.partial"));
    std::fs::write(&partial, body.to_string()).map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    std::fs::rename(&partial, &path).map_err(|e| format!("cannot save {}: {e}", path.display()))?;
    for old in list(dir, key).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(path)
}

/// The project's chats, newest first.
pub fn list(dir: &Path, key: &str) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(dir.join(key)) else { return Vec::new() };
    let mut out: Vec<Saved> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| {
            let body: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            Some(Saved {
                title: body["title"].as_str().unwrap_or("(untitled)").to_string(),
                saved_at: body["saved_at"].as_u64().unwrap_or(0),
                messages: body["messages"].as_array().map_or(0, Vec::len),
                path,
            })
        })
        .collect();
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(b.path.cmp(&a.path)));
    out
}

/// A saved chat's messages.
pub fn load(path: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let body: Value = serde_json::from_str(&text).map_err(|e| format!("{} is damaged: {e}", path.display()))?;
    body["messages"].as_array().cloned().ok_or_else(|| format!("{} holds no messages", path.display()))
}

/// The chat's id, from its file.
pub fn id_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// Pure — unit-tested. "5 minutes ago", for the list.
pub fn age(saved_at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(saved_at);
    let (n, unit) = match seconds {
        0..60 => return "just now".to_string(),
        60..3600 => (seconds / 60, "minute"),
        3600..86400 => (seconds / 3600, "hour"),
        _ => (seconds / 86400, "day"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Seconds since 1970, now.
pub fn now_seconds() -> u64 {
    now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    #[test]
    fn projects_get_their_own_folder_and_chats_a_title() {
        let a = project_key(Path::new("/Users/me/work/App"));
        assert!(a.starts_with("app-") && a.len() == "app-".len() + 8, "{a}");
        assert_ne!(a, project_key(Path::new("/Users/me/other/App")));
        assert!(project_key(Path::new("/")).starts_with("root-"));
        let history = vec![json!({ "role": "user", "content": "fix the\nlogin bug" }), json!({ "role": "assistant", "content": "ok" })];
        assert_eq!(title(&history), "fix the login bug");
        assert_eq!(age(100, 130), "just now");
        assert_eq!(age(0, 7200), "2 hours ago");
        assert_eq!(age(0, 86400), "1 day ago");
    }

    #[test]
    fn chats_are_saved_listed_newest_first_loaded_and_pruned() {
        let dir = temp_dir("sessions");
        let key = "app-00000000";
        let chat = |text: &str| vec![json!({ "role": "user", "content": text }), json!({ "role": "assistant", "content": "done" })];
        let first = save(&dir, key, "1-000", &chat("first"), "m").unwrap();
        save(&dir, key, "2-000", &chat("second"), "m").unwrap();
        // Saving again under the same id replaces that chat.
        save(&dir, key, "2-000", &[chat("second"), chat("more")].concat(), "m").unwrap();
        let listed = list(&dir, key);
        assert_eq!(listed.len(), 2);
        assert_eq!((listed[0].title.as_str(), listed[0].messages), ("second", 4));
        assert_eq!(load(&first).unwrap(), chat("first"));
        assert_eq!(id_of(&first), "1-000");
        assert!(list(&dir, "other").is_empty());

        for n in 0..KEEP + 3 {
            save(&dir, key, &format!("9{n:03}-000"), &chat("x"), "m").unwrap();
        }
        assert_eq!(list(&dir, key).len(), KEEP);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
