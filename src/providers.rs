//! The model providers rak knows by name: where each one is, which key it
//! takes, and which API it speaks. The list is data (providers.json, compiled
//! in), so adding a provider is a line there, not code.

use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    /// What `--provider` and RAK_PROVIDER take, e.g. "openrouter".
    pub name: String,
    /// For people, e.g. "OpenRouter".
    pub label: String,
    pub base_url: String,
    /// The variable that holds its key; None for servers that take none.
    pub key: Option<String>,
    /// Anthropic's Messages API rather than OpenAI's chat-completions API.
    pub anthropic: bool,
}

const DATA: &str = include_str!("providers.json");

/// Every provider, in the order of providers.json.
pub fn all() -> &'static [Provider] {
    static ALL: OnceLock<Vec<Provider>> = OnceLock::new();
    ALL.get_or_init(|| parse(DATA).expect("providers.json is valid (a unit test checks it)"))
}

/// The provider with this name, ignoring case.
pub fn find(name: &str) -> Option<&'static Provider> {
    all().iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

/// The names, for messages: "ollama, ollama-cloud, anthropic, …".
pub fn names() -> String {
    all().iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
}

/// Pure — unit-tested.
pub fn parse(text: &str) -> Result<Vec<Provider>, String> {
    let list: Vec<Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    list.iter()
        .map(|p| {
            let field = |name: &str| p[name].as_str().map(str::to_string).ok_or(format!("a provider has no {name}: {p}"));
            let api = field("api")?;
            if api != "openai" && api != "anthropic" {
                return Err(format!("api is openai or anthropic, not {api}"));
            }
            Ok(Provider {
                name: field("name")?,
                label: field("label")?,
                base_url: field("base_url")?,
                key: p["key"].as_str().map(str::to_string),
                anthropic: api == "anthropic",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_parses_and_names_are_unique() {
        let all = parse(DATA).unwrap();
        assert!(all.len() >= 10);
        let mut names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "a name is listed twice");
        assert!(all.iter().all(|p| p.base_url.starts_with("http") && !p.base_url.ends_with('/')));
        assert_eq!(find("OpenRouter").unwrap().key.as_deref(), Some("OPENROUTER_API_KEY"));
        assert!(find("anthropic").unwrap().anthropic);
        assert_eq!(find("ollama").unwrap().key, None);
        assert_eq!(find("ollama-cloud").unwrap().key.as_deref(), Some("OLLAMA_API_KEY"));
        assert!(find("nope").is_none());
    }

    #[test]
    fn a_bad_entry_is_reported() {
        assert!(parse(r#"[{ "name": "x", "label": "X", "base_url": "http://x", "api": "grpc" }]"#).is_err());
        assert!(parse(r#"[{ "name": "x", "api": "openai" }]"#).unwrap_err().contains("no label"));
    }
}
