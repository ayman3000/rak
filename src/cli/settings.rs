//! What rak keeps between runs (the folder, the model picked last), and the
//! keys it reads from the environment and .env files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where rak keeps the history and the model picked last.
pub fn config_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config"))).or_else(|| var("APPDATA")).map(|base| base.join("rak"))
}

pub fn home() -> String {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()
}

pub fn saved_model() -> Option<String> {
    let text = std::fs::read_to_string(config_dir()?.join("model")).ok()?;
    Some(text.trim().to_string()).filter(|m| !m.is_empty())
}

pub fn save_model(model: &str) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("model"), format!("{model}\n"));
    }
}

// MARK: - Keys and .env files

/// Settings and API keys, like python-dotenv. Looked up in this order, the
/// first that has the key wins:
///   1. the real environment (export TAVILY_API_KEY=… in the shell)
///   2. ./.env              in the project folder
///   3. ~/.config/rak/.env  (keys for every project)
///
/// Values from the files stay inside rak and are not exported, so commands the
/// agent runs with run_shell never see them.
#[derive(Default)]
pub struct Keys {
    values: HashMap<String, String>,
    /// The .env files read, project first.
    pub files: Vec<PathBuf>,
}

impl Keys {
    pub fn load(project: &Path) -> Keys {
        let mut keys = Keys::default();
        // Lowest priority first, so later files overwrite earlier ones.
        let global = config_dir().map(|dir| dir.join(".env"));
        for file in global.into_iter().chain([project.join(".env")]) {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            keys.values.extend(parse_env(&text));
            keys.files.insert(0, file);
        }
        keys
    }

    pub fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().or_else(|| self.values.get(name).cloned()).filter(|v| !v.is_empty())
    }
}

static KEYS: OnceLock<Keys> = OnceLock::new();

/// Reads the .env files for this project. Once, before the first `key()`.
pub fn load_keys(project: &Path) {
    let _ = KEYS.set(Keys::load(project));
}

/// A setting or key; the environment alone until `load_keys` has run.
pub fn key(name: &str) -> Option<String> {
    match KEYS.get() {
        Some(keys) => keys.get(name),
        None => std::env::var(name).ok().filter(|v| !v.is_empty()),
    }
}

/// The .env files that were read, project first.
pub fn key_files() -> Vec<PathBuf> {
    KEYS.get().map(|k| k.files.clone()).unwrap_or_default()
}

/// Pure — unit-tested. KEY=VALUE lines: `#` comments, blank lines, an
/// optional `export ` prefix, single- or double-quoted values, and a ` #`
/// comment after an unquoted value.
pub fn parse_env(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export ") {
            line = rest.trim_start();
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let mut value = value.trim();
        let quoted = value.len() >= 2 && (value.starts_with('"') || value.starts_with('\''));
        if quoted {
            let quote = &value[..1];
            if let Some(end) = value[1..].find(quote) {
                value = &value[1..1 + end];
            }
        } else if let Some(hash) = value.find(" #") {
            value = value[..hash].trim_end();
        }
        if !key.is_empty() {
            out.insert(key.to_string(), value.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_files_are_read_like_dotenv() {
        let parsed = parse_env(
            "# keys\n\nTAVILY_API_KEY=tvly-1\nexport ANTHROPIC_API_KEY = \"sk-ant x\"\nQUOTED='a # b'\nPLAIN=value # comment\nNOEQUALS\nEMPTY=\n",
        );
        assert_eq!(parsed["TAVILY_API_KEY"], "tvly-1");
        assert_eq!(parsed["ANTHROPIC_API_KEY"], "sk-ant x");
        assert_eq!(parsed["QUOTED"], "a # b");
        assert_eq!(parsed["PLAIN"], "value");
        assert_eq!(parsed["EMPTY"], "");
        assert!(!parsed.contains_key("NOEQUALS"));
    }

    #[test]
    fn the_project_file_wins_over_the_global_one_and_empty_values_count_as_unset() {
        let mut keys = Keys::default();
        keys.values.extend(parse_env("A=global\nB=global\n"));
        keys.values.extend(parse_env("A=project\nC=\n"));
        assert_eq!(keys.get("A").as_deref(), Some("project"));
        assert_eq!(keys.get("B").as_deref(), Some("global"));
        assert_eq!(keys.get("C"), None);
        assert_eq!(keys.get("RAK_TEST_SURELY_UNSET_KEY"), None);
    }
}
