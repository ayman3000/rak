//! Tool outputs removed to save context, kept so the model can read them back
//! instead of running the tool again. From SwiftAgentKit's `ArtifactStore`,
//! `artifact_read` and `artifact_search`, made smaller: the store lives in
//! memory for the chat (a saved chat does not keep it), and only outputs that
//! were actually removed go in.
//!
//! The two tools join the model's list only once something has been stored,
//! so a conversation that never fills its window pays nothing for them.

use crate::read_slice::FLOOR;
use crate::text::one_line;
use serde_json::{json, Value};

/// Everything kept, at most: past this the oldest outputs are let go.
const MAX_TOTAL_CHARS: usize = 4_000_000;
/// What `artifact_read` returns when no limit is given: usually all of it.
const DEFAULT_READ: usize = 24_000;
const CONTEXT_LINES: usize = 2;

pub const READ: &str = "artifact_read";
pub const SEARCH: &str = "artifact_search";

#[derive(Default)]
pub struct Artifacts {
    /// (id, what it was, the text), oldest first.
    items: Vec<(String, String, String)>,
    next: usize,
}

impl Artifacts {
    /// Keeps `text`; returns its id ("out-3").
    pub fn save(&mut self, label: &str, text: &str) -> String {
        self.next += 1;
        let id = format!("out-{}", self.next);
        self.items.push((id.clone(), label.to_string(), text.to_string()));
        let mut total: usize = self.items.iter().map(|(_, _, t)| t.len()).sum();
        while total > MAX_TOTAL_CHARS && self.items.len() > 1 {
            total -= self.items.remove(0).2.len();
        }
        id
    }

    /// Undoes the latest `save` (its output stayed in the conversation).
    pub fn forget_last(&mut self) {
        self.items.pop();
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    fn get(&self, id: &str) -> Option<&(String, String, String)> {
        self.items.iter().find(|(i, _, _)| i == id.trim().trim_matches(['[', ']']))
    }

    /// Runs `artifact_read` or `artifact_search`: (ok, text).
    pub fn run(&self, tool: &str, args: &Value) -> (bool, String) {
        let Some(id) = args["artifact_id"].as_str().filter(|s| !s.is_empty()) else {
            return (false, format!("{tool} requires an artifact_id (the out-N in a removed output's note)."));
        };
        let Some((id, label, text)) = self.get(id) else {
            return (false, format!("No stored output {id}: it was not kept (or this chat was resumed). Run the tool again."));
        };
        if tool == READ {
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().map(|l| l as usize).unwrap_or(DEFAULT_READ);
            return (true, read(text, offset, limit));
        }
        let mut queries: Vec<String> = args["queries"]
            .as_array()
            .map(|q| q.iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        if let Some(one) = args["query"].as_str().filter(|s| !s.is_empty()) {
            queries.push(one.to_string());
        }
        if queries.is_empty() {
            return (false, "artifact_search requires `queries` (a list) or `query`.".to_string());
        }
        let max = args["max_matches"].as_u64().map(|m| m.max(1) as usize).unwrap_or(10);
        (true, format!("{id} ({label}):\n{}", search(text, &queries, max)))
    }
}

/// Pure — unit-tested. A slice of `text` from character `offset`; a tiny
/// limit is widened, as read_file does: every call re-sends the conversation.
pub fn read(text: &str, offset: usize, limit: usize) -> String {
    let total = text.chars().count();
    if offset >= total {
        return format!("[offset {offset} is past the end: the output has {total} characters]");
    }
    let limit = limit.max(FLOOR);
    let slice: String = text.chars().skip(offset).take(limit).collect();
    let end = offset + slice.chars().count();
    if end < total {
        format!("{slice}\n… [characters {offset}–{end} of {total}; call again with offset {end} for more]")
    } else {
        slice
    }
}

/// Pure — unit-tested. The lines holding each query (any case), two lines of
/// context around each, numbered.
pub fn search(text: &str, queries: &[String], max_per_query: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let lower: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
    let mut sections = Vec::new();
    for query in queries {
        let needle = query.to_lowercase();
        let hits: Vec<usize> = lower.iter().enumerate().filter(|(_, l)| l.contains(&needle)).map(|(i, _)| i).take(max_per_query).collect();
        if hits.is_empty() {
            sections.push(format!("\"{query}\": no matches"));
            continue;
        }
        let blocks: Vec<String> = hits
            .iter()
            .map(|&hit| {
                let (lo, hi) = (hit.saturating_sub(CONTEXT_LINES), (hit + CONTEXT_LINES).min(lines.len() - 1));
                (lo..=hi)
                    .map(|n| format!("{}{} | {}", if n == hit { ">" } else { " " }, n + 1, one_line(lines[n], 300)))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect();
        sections.push(format!("\"{query}\": {} match{}\n{}", hits.len(), if hits.len() == 1 { "" } else { "es" }, blocks.join("\n…\n")));
    }
    sections.join("\n\n")
}

/// The two tools' definitions, sent once something is stored.
pub fn definitions() -> Vec<Value> {
    vec![
        json!({ "type": "function", "function": {
            "name": READ,
            "description": "Read a tool output that was removed to save context, by the id in its note (out-N). \
                Usually returns all of it; use `offset` to page through a very long one.",
            "parameters": { "type": "object", "properties": {
                "artifact_id": { "type": "string", "description": "The id from the note, e.g. out-3." },
                "offset": { "type": "integer", "description": "Character to start at (default 0)." },
                "limit": { "type": "integer", "description": "Characters to return (default 24000)." }
            }, "required": ["artifact_id"] }
        }}),
        json!({ "type": "function", "function": {
            "name": SEARCH,
            "description": "Search a removed tool output for lines containing words, with two lines of context \
                each. Pass every term in one call via `queries` (e.g. [\"error:\", \"FAILED\"]).",
            "parameters": { "type": "object", "properties": {
                "artifact_id": { "type": "string", "description": "The id from the note, e.g. out-3." },
                "queries": { "type": "array", "items": { "type": "string" }, "description": "Words to look for, any case." },
                "max_matches": { "type": "integer", "description": "Matches per query (default 10)." }
            }, "required": ["artifact_id", "queries"] }
        }}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_are_kept_by_id_and_the_oldest_go_past_the_cap() {
        let mut store = Artifacts::default();
        let first = store.save("read_file a.rs", "fn a() {}");
        assert_eq!(first, "out-1");
        let (ok, text) = store.run(READ, &json!({ "artifact_id": "[out-1]" }));
        assert!(ok && text == "fn a() {}", "{text}");
        store.save("big", &"x".repeat(MAX_TOTAL_CHARS));
        assert!(!store.run(READ, &json!({ "artifact_id": "out-1" })).0, "let go past the cap");
        assert!(store.run(READ, &json!({ "artifact_id": "out-2" })).0, "the newest stays even when large");
        assert!(store.run(READ, &json!({ "artifact_id": "out-9" })).1.contains("Run the tool again"));
    }

    #[test]
    fn a_long_output_is_read_in_slices_and_a_tiny_one_widened() {
        let text = "a".repeat(10_000);
        let page = read(&text, 0, 10);
        assert!(page.starts_with(&"a".repeat(FLOOR)) && page.contains("call again with offset 4000"), "widened to the floor");
        assert!(read(&text, 9_000, 50_000).ends_with('a'));
        assert!(read(&text, 20_000, 10).contains("past the end"));
    }

    #[test]
    fn search_shows_each_match_with_its_lines_around_it() {
        let log = "compiling\nok\nerror: missing ;\nat line 4\nwarning: unused\ndone";
        let found = search(log, &["ERROR".into(), "nothing".into()], 10);
        assert_eq!(found, "\"ERROR\": 1 match\n 1 | compiling\n 2 | ok\n>3 | error: missing ;\n 4 | at line 4\n 5 | warning: unused\n\n\"nothing\": no matches");
        let mut store = Artifacts::default();
        store.save("run_shell cargo build", log);
        let (ok, text) = store.run(SEARCH, &json!({ "artifact_id": "out-1", "query": "warning" }));
        assert!(ok && text.starts_with("out-1 (run_shell cargo build):") && text.contains(">5 | warning"), "{text}");
        assert!(!store.run(SEARCH, &json!({ "artifact_id": "out-1" })).0);
    }
}
