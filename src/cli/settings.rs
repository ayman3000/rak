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

/// The last model picked for each provider, kept in ~/.config/rak/models as
/// one `provider=model` line each (a server named only by its address is kept
/// under that address).
fn models_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("models"))
}

/// Pure — unit-tested.
pub fn parse_models(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

/// Pure — unit-tested. The list with this provider's pick replaced or added.
pub fn with_model(mut list: Vec<(String, String)>, provider: &str, model: &str) -> Vec<(String, String)> {
    match list.iter_mut().find(|(k, _)| k == provider) {
        Some(entry) => entry.1 = model.to_string(),
        None => list.push((provider.to_string(), model.to_string())),
    }
    list
}

/// The model last picked for this provider.
pub fn last_model(provider: &str) -> Option<String> {
    let text = std::fs::read_to_string(models_file()?).unwrap_or_default();
    parse_models(&text).into_iter().find(|(k, _)| k == provider).map(|(_, v)| v).or_else(|| {
        // Before picks were kept per provider there was one, made on Ollama.
        (provider == "ollama")
            .then(|| std::fs::read_to_string(config_dir()?.join("model")).ok())
            .flatten()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
    })
}

/// Remembers the model picked for this provider.
pub fn remember_model(provider: &str, model: &str) {
    let Some(file) = models_file() else { return };
    let list = parse_models(&std::fs::read_to_string(&file).unwrap_or_default());
    let text: String = with_model(list, provider, model).iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, text);
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
    /// Name → (value, the file it came from).
    values: HashMap<String, (String, PathBuf)>,
    /// The .env files read, project first.
    pub files: Vec<PathBuf>,
}

/// The template for ~/.config/rak/.env: the keys rak uses, explained.
pub const TEMPLATE: &str = include_str!("../../.env.example");

/// Settings `rak keys` reports on besides the providers' keys, and what each
/// is for. The defaults are not secrets, so their values are shown.
pub const KNOWN: &[(&str, &str)] = &[
    ("TAVILY_API_KEY", "web_search"),
    ("RAK_PROVIDER", "the default provider"),
    ("RAK_MODEL", "the default model"),
    ("RAK_BASE_URL", "the default provider's address, if not its usual one"),
    ("RAK_API_KEY", "the key for a server named only by --base-url"),
];

/// Which of `KNOWN` are settings whose values may be shown.
pub fn shows_value(name: &str) -> bool {
    matches!(name, "RAK_PROVIDER" | "RAK_MODEL" | "RAK_BASE_URL")
}

/// rak's own key file: ~/.config/rak/.env.
pub fn global_key_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(".env"))
}

impl Keys {
    pub fn load(project: &Path) -> Keys {
        let mut keys = Keys::default();
        // Lowest priority first, so later files overwrite earlier ones.
        for file in global_key_file().into_iter().chain([project.join(".env")]) {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            keys.values.extend(parse_env(&text).into_iter().map(|(k, v)| (k, (v, file.clone()))));
            keys.files.insert(0, file);
        }
        keys
    }

    pub fn get(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| self.values.get(name).map(|(v, _)| v.clone()))
            .filter(|v| !v.is_empty())
    }

    /// Where a key that is set comes from: "the environment" or a file.
    pub fn source(&self, name: &str) -> Option<String> {
        if std::env::var(name).is_ok_and(|v| !v.is_empty()) {
            return Some("the environment".to_string());
        }
        self.values.get(name).filter(|(v, _)| !v.is_empty()).map(|(_, file)| file.display().to_string())
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
        } else if value.starts_with('#') {
            value = ""; // `KEY=   # what it is for`: empty, with a comment
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
        let from = |text: &str, file: &str| parse_env(text).into_iter().map(|(k, v)| (k, (v, PathBuf::from(file)))).collect::<Vec<_>>();
        keys.values.extend(from("A=global\nB=global\n", "g"));
        keys.values.extend(from("A=project\nC=\n", "p"));
        assert_eq!(keys.get("A").as_deref(), Some("project"));
        assert_eq!(keys.get("B").as_deref(), Some("global"));
        assert_eq!(keys.get("C"), None);
        assert_eq!(keys.get("RAK_TEST_SURELY_UNSET_KEY"), None);
        assert_eq!(keys.source("A").as_deref(), Some("p"));
        assert_eq!(keys.source("C"), None, "empty is not set");
    }

    #[test]
    fn picks_are_kept_per_provider() {
        let list = parse_models("ollama=kimi-k2.6:cloud\nopenrouter = x/y\nbroken\nempty=\n");
        assert_eq!(list, [("ollama".to_string(), "kimi-k2.6:cloud".to_string()), ("openrouter".into(), "x/y".into())]);
        let list = with_model(list, "openrouter", "x/z");
        let list = with_model(list, "https://example.com/v1", "m");
        assert_eq!(list[1].1, "x/z");
        assert_eq!(list[2], ("https://example.com/v1".to_string(), "m".to_string()));
        assert_eq!(parse_models("https://example.com/v1=m\n")[0].0, "https://example.com/v1", "an address keeps its colon");
    }

    #[test]
    fn an_empty_value_followed_by_a_comment_is_empty() {
        let parsed = parse_env("A=      # the key for A\nB=b   # the key for B\nC='#not a comment'\n");
        assert_eq!(parsed["A"], "");
        assert_eq!(parsed["B"], "b");
        assert_eq!(parsed["C"], "#not a comment");
    }

    #[test]
    fn the_template_lists_every_known_key_and_sets_none() {
        let parsed = parse_env(TEMPLATE);
        for (name, _) in KNOWN {
            assert!(TEMPLATE.contains(name), "{name} is missing from .env.example");
        }
        for provider in rak::providers::all() {
            assert!(TEMPLATE.contains(&format!("RAK_PROVIDER={}\n", provider.name)), "no block for {} in .env.example", provider.name);
            if let Some(key) = &provider.key {
                assert!(parsed.contains_key(key), "{key} ({}) has no line in .env.example", provider.name);
            }
        }
        assert!(parsed.values().all(String::is_empty), "the template must not set anything: {parsed:?}");
    }
}
