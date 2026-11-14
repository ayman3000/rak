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
    /// Settings set more than once in one file, and which value is used.
    pub notes: Vec<String>,
    /// Names given a value, in the order they appear: the project's .env
    /// first, then the global file. The fallback chain follows it.
    pub order: Vec<String>,
}

/// Pure — unit-tested. Names given a (non-empty) value, in line order, once each.
pub fn assigned_in_order(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some((name, value)) = parse_env(line).into_iter().next() {
            if !value.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Settings whose values may be named in a note: not secrets.
const SHOWN: &[&str] = &["RAK_PROVIDER", "RAK_MODEL", "RAK_BASE_URL"];

/// Pure — unit-tested. Names given a value more than once in a .env text:
/// (name, how many times, the value used — the last one set).
pub fn repeated(text: &str) -> Vec<(String, usize, String)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = parse_env(line).into_iter().next() else { continue };
        if value.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 += 1,
            None => counts.push((key, 1)),
        }
    }
    let used = parse_env(text);
    counts.into_iter().filter(|(_, n)| *n > 1).map(|(k, n)| (k.clone(), n, used[&k].clone())).collect()
}

/// Pure — unit-tested. The note for a name set more than once.
pub fn repeated_note(name: &str, times: usize, used: &str, file: &str) -> String {
    if SHOWN.contains(&name) {
        format!("{name} is set {times} times in {file}: the last one, {used}, is used")
    } else {
        format!("{name} is set {times} times in {file}: the last one is used")
    }
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
    /// `with_project`: also the project's .env (only in a trusted folder).
    pub fn load(project: &Path, with_project: bool) -> Keys {
        let mut keys = Keys::default();
        // Lowest priority first, so later files overwrite earlier ones.
        let project_file = with_project.then(|| project.join(".env"));
        for file in global_key_file().into_iter().chain(project_file) {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            // An empty line never overrides a value from another file.
            keys.values.extend(parse_env(&text).into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k, (v, file.clone()))));
            let (path, home) = (file.to_string_lossy().to_string(), home());
            let shown = if home.is_empty() { path } else { path.replacen(&home, "~", 1) };
            keys.notes.extend(repeated(&text).into_iter().map(|(k, n, used)| repeated_note(&k, n, &used, &shown)));
            // The project's file is read last but comes first in the order.
            let mut order = assigned_in_order(&text);
            order.extend(keys.order.drain(..).filter(|name| !order.contains(name)).collect::<Vec<_>>());
            keys.order = order;
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

/// Reads the .env files for this project (its own only when trusted). Once,
/// before the first `key()`.
pub fn load_keys(project: &Path, with_project: bool) {
    let _ = KEYS.set(Keys::load(project, with_project));
}

/// A setting or key; the environment alone until `load_keys` has run.
pub fn key(name: &str) -> Option<String> {
    match KEYS.get() {
        Some(keys) => keys.get(name),
        None => std::env::var(name).ok().filter(|v| !v.is_empty()),
    }
}

/// Names given a value in the .env files, in the order they appear.
pub fn key_order() -> Vec<String> {
    KEYS.get().map(|k| k.order.clone()).unwrap_or_default()
}

/// Settings set more than once in a .env file, and which value is used.
pub fn key_notes() -> Vec<String> {
    KEYS.get().map(|k| k.notes.clone()).unwrap_or_default()
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
        if key.is_empty() {
            continue;
        }
        // The last value set wins; a later empty line (the template's own
        // `RAK_PROVIDER=`, say) does not erase an earlier one.
        if value.is_empty() {
            out.entry(key.to_string()).or_default();
        } else {
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
    fn the_last_value_set_wins_and_an_empty_line_erases_nothing() {
        let text = "RAK_PROVIDER=anthropic\nRAK_MODEL=claude-sonnet-5-5\nRAK_PROVIDER=zai\nRAK_MODEL=glm-5.2\nRAK_PROVIDER=\nOPENAI_API_KEY=a\nOPENAI_API_KEY=b\nEMPTY=\n";
        let parsed = parse_env(text);
        assert_eq!((parsed["RAK_PROVIDER"].as_str(), parsed["RAK_MODEL"].as_str()), ("zai", "glm-5.2"));
        assert_eq!(parsed["OPENAI_API_KEY"], "b");
        assert_eq!(parsed["EMPTY"], "", "an empty line still counts as present");
        let notes: Vec<String> = repeated(text).into_iter().map(|(k, n, used)| repeated_note(&k, n, &used, "~/.config/rak/.env")).collect();
        assert_eq!(
            notes,
            [
                "RAK_PROVIDER is set 2 times in ~/.config/rak/.env: the last one, zai, is used",
                "RAK_MODEL is set 2 times in ~/.config/rak/.env: the last one, glm-5.2, is used",
                "OPENAI_API_KEY is set 2 times in ~/.config/rak/.env: the last one is used",
            ]
        );
        assert!(notes.iter().all(|n| !n.contains("=b") && !n.contains(" b,")), "a key's value is never shown");
    }

    #[test]
    fn names_are_listed_in_the_order_they_are_set() {
        let text = "# keys\nGROQ_API_KEY=g\nOPENAI_API_KEY=\nANTHROPIC_API_KEY=a\nGROQ_API_KEY=g2\nRAK_PROVIDER=zai\n";
        assert_eq!(assigned_in_order(text), ["GROQ_API_KEY", "ANTHROPIC_API_KEY", "RAK_PROVIDER"]);
    }

    #[test]
    fn an_empty_line_in_one_file_does_not_hide_a_key_from_another() {
        let mut keys = Keys::default();
        for (text, file) in [("A=global\n", "g"), ("A=\n", "p")] {
            keys.values.extend(parse_env(text).into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k, (v, PathBuf::from(file)))));
        }
        assert_eq!(keys.get("A").as_deref(), Some("global"));
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
        // The settings have plain lines of their own, so they are filled in, not uncommented.
        for setting in ["RAK_PROVIDER", "RAK_MODEL", "RAK_BASE_URL", "RAK_API_KEY"] {
            assert!(parsed.contains_key(setting), "{setting} has no line in .env.example");
        }
        let provider_list = TEMPLATE.split("RAK_PROVIDER is one of:").nth(1).unwrap().split("RAK_PROVIDER=").next().unwrap();
        for provider in rak::providers::all() {
            let named = provider_list.split(|c: char| c == ',' || c.is_whitespace()).any(|word| word == provider.name);
            assert!(named, "{} is missing from the RAK_PROVIDER list in .env.example", provider.name);
            if let Some(key) = &provider.key {
                assert!(parsed.contains_key(key), "{key} ({}) has no line in .env.example", provider.name);
            }
        }
        assert!(parsed.values().all(String::is_empty), "the template must not set anything: {parsed:?}");
    }
}
