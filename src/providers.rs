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
    /// A ChatGPT plan through Sign in with ChatGPT (`rak login chatgpt`): OpenAI's
    /// Responses API with the sign-in's token, no key.
    pub chatgpt: bool,
    /// An address a key must be good to read (GET, the key as a bearer
    /// token), for providers whose model list answers anyone: setup checks
    /// the key there, so a mistyped one is caught before the first message.
    pub key_check: Option<String>,
}

const DATA: &str = include_str!("providers.json");

/// What "is its key set?" is asked with for a provider that is signed in to
/// rather than given a key (the `chatgpt` one).
pub const CHATGPT_SIGN_IN: &str = "the ChatGPT sign-in";

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
            if !["openai", "anthropic", "chatgpt"].contains(&api.as_str()) {
                return Err(format!("api is openai, anthropic or chatgpt, not {api}"));
            }
            Ok(Provider {
                name: field("name")?,
                label: field("label")?,
                base_url: field("base_url")?,
                key: p["key"].as_str().map(str::to_string),
                anthropic: api == "anthropic",
                chatgpt: api == "chatgpt",
                key_check: p["key_check"].as_str().map(str::to_string),
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
        let chatgpt = find("chatgpt").unwrap();
        assert!(chatgpt.chatgpt && chatgpt.key.is_none() && !chatgpt.anthropic, "signed in to, not keyed");
        assert_eq!(all.iter().filter(|p| p.chatgpt).count(), 1);
        assert_eq!(find("ollama-cloud").unwrap().key.as_deref(), Some("OLLAMA_API_KEY"));
        assert!(find("nope").is_none());
        // Where the model list answers without a key, a key check is listed.
        // (ollama.com has none known: its /api/ps refuses a good key too.)
        for name in ["openrouter", "huggingface"] {
            assert!(find(name).unwrap().key_check.as_deref().is_some_and(|u| u.starts_with("https://")), "{name}");
        }
        assert!(all.iter().filter(|p| p.key_check.is_some()).all(|p| p.key.is_some()), "a check needs a key");
    }

    #[test]
    fn a_bad_entry_is_reported() {
        assert!(parse(r#"[{ "name": "x", "label": "X", "base_url": "http://x", "api": "grpc" }]"#).is_err());
        assert!(parse(r#"[{ "name": "x", "api": "openai" }]"#).unwrap_err().contains("no label"));
    }
}
