//! Routines: a task the agent has done once, saved as fixed steps that run
//! back to back with no model call between them. From SwiftAgentKit's
//! `Routine`, `run_routine` and `save_routine`, in the same JSON shape, so
//! sak, Naseem and rak read one another's routines.
//!
//! The point is not that the steps are written down — a skill does that, and
//! the model still pays a turn per step reading it. A routine runs without
//! asking a model anything: a task that cost a dozen calls costs one.
//!
//! Deliberately not a language: inputs are values put into `{placeholders}`,
//! never expressions; steps run in order with no branching. A step that fails
//! stops the routine and hands control back. Every step goes through the same
//! approval as a call the model makes itself (the agent runs them).

use crate::tools::{str_arg, Tool, ToolResult};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const RUN: &str = "run_routine";
const FOLDER: &str = "routines";

/// The project's routines folder and the one for every project.
pub fn folders(project: &Path, config_dir: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    (project.join(".rak").join(FOLDER), config_dir.map(|d| d.join(FOLDER)))
}

/// Pure — unit-tested. A name safe as a file name ("Note to Word" → "Note-to-Word").
pub fn file_safe(name: &str) -> String {
    let cleaned: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    cleaned.trim_matches('-').to_string()
}

/// Every routine in these folders (the first folder wins a name), by name.
pub fn all(folders: &[PathBuf]) -> Vec<Value> {
    let mut found: Vec<Value> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for file in files {
            let Some(routine) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
            let Some(name) = routine["name"].as_str() else { continue };
            if routine["steps"].as_array().is_some_and(|s| !s.is_empty()) && !found.iter().any(|r| r["name"] == name) {
                found.push(routine);
            }
        }
    }
    found.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    found
}

/// Pure — unit-tested. `{name}` in every string of `value` becomes its value;
/// unknown placeholders stay, so a step visibly fails rather than act on a blank.
pub fn fill(value: &Value, values: &[(String, String)]) -> Value {
    match value {
        Value::String(s) => {
            let mut out = s.clone();
            for (key, v) in values {
                out = out.replace(&format!("{{{key}}}"), v);
            }
            Value::String(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, values)).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), fill(v, values))).collect()),
        other => other.clone(),
    }
}

/// Pure — unit-tested. Placeholders the steps use that no input declares.
pub fn undeclared(routine: &Value) -> Vec<String> {
    fn scan(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::String(s) => {
                let mut rest = s.as_str();
                while let Some(open) = rest.find('{') {
                    let Some(close) = rest[open..].find('}') else { break };
                    let key = &rest[open + 1..open + close];
                    if !key.is_empty() && !key.contains([' ', '"', ':', '{']) && !found.iter().any(|f| f == key) {
                        found.push(key.to_string());
                    }
                    rest = &rest[open + close + 1..];
                }
            }
            Value::Array(items) => items.iter().for_each(|v| scan(v, found)),
            Value::Object(map) => map.values().for_each(|v| scan(v, found)),
            _ => {}
        }
    }
    let mut used = Vec::new();
    for step in routine["steps"].as_array().into_iter().flatten() {
        scan(&step["arguments"], &mut used);
    }
    let declared: Vec<&str> = routine["inputs"].as_array().into_iter().flatten().filter_map(|i| i["name"].as_str()).collect();
    let mut missing: Vec<String> = used.into_iter().filter(|u| !declared.contains(&u.as_str())).collect();
    missing.sort();
    missing
}

/// Pure — unit-tested. The values for a run, or what is missing.
pub fn inputs_for(routine: &Value, supplied: &Value) -> Result<Vec<(String, String)>, Vec<String>> {
    let (mut values, mut missing) = (Vec::new(), Vec::new());
    for input in routine["inputs"].as_array().into_iter().flatten() {
        let Some(name) = input["name"].as_str() else { continue };
        let given = match &supplied[name] {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        };
        match given.or_else(|| input["defaultValue"].as_str().map(String::from)) {
            Some(v) => values.push((name.to_string(), v)),
            None if input["required"].as_bool().unwrap_or(true) => {
                missing.push(format!("{name} ({})", input["description"].as_str().unwrap_or("")))
            }
            None => values.push((name.to_string(), String::new())),
        }
    }
    if missing.is_empty() {
        Ok(values)
    } else {
        Err(missing)
    }
}

/// The run_routine definition, listing the routines saved.
pub fn run_definition(routines: &[Value]) -> Value {
    let listed: Vec<String> = routines
        .iter()
        .map(|r| {
            let inputs: Vec<String> = r["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| {
                    i["name"].as_str().map(|n| format!("{n}{}", if i["required"].as_bool().unwrap_or(true) { "" } else { "?" }))
                })
                .collect();
            let inputs = if inputs.is_empty() { "no inputs".to_string() } else { inputs.join(", ") };
            format!("  • {} — {} [{inputs}]", r["name"].as_str().unwrap_or(""), r["description"].as_str().unwrap_or(""))
        })
        .collect();
    json!({ "type": "function", "function": {
        "name": RUN,
        "description": format!(
            "Run a saved routine: proven steps that run back to back with NO model call between them, so a task \
             that would cost a dozen steps costs one. Prefer it whenever a routine covers the request; supply its \
             inputs from what the person asked. A step that fails stops the routine and says which; you take over \
             from there. Each step that changes something still asks for approval.\n\nSaved routines:\n{}",
            listed.join("\n")
        ),
        "parameters": { "type": "object", "properties": {
            "name": { "type": "string", "description": "Which saved routine to run." },
            "inputs": { "type": "object", "description": "Values for its inputs, e.g. {\"version\": \"1.2.0\"}." }
        }, "required": ["name"] }
    }})
}

/// Marks a routine as having completed end to end (`lastSucceededAt`).
pub fn mark_succeeded(folders: &[PathBuf], name: &str) {
    for folder in folders {
        let file = folder.join(format!("{}.json", file_safe(name)));
        let Some(mut routine) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
        routine["lastSucceededAt"] = json!(iso_now());
        if let Ok(text) = serde_json::to_string_pretty(&routine) {
            let _ = std::fs::write(&file, text);
        }
        return;
    }
}

/// Now as ISO 8601 in UTC ("2026-11-13T09:30:00Z"), as Swift's encoder writes it.
pub fn iso_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    iso(secs)
}

/// Pure — unit-tested. Seconds since 1970 as ISO 8601 in UTC.
pub fn iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// save_routine: keeps a sequence that just worked.
pub struct SaveRoutine {
    pub project: PathBuf,
    pub user: Option<PathBuf>,
}

impl Tool for SaveRoutine {
    fn name(&self) -> &str {
        "save_routine"
    }
    fn description(&self) -> &str {
        "Save steps that just ran and worked as a routine, to run later with no model call between them. What \
         varies becomes a {placeholder} declared in `inputs`. Asks first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "kebab-case" },
                "description": { "type": "string" },
                "inputs": { "type": "array", "description": "[{name, description, required?, default?}]", "items": { "type": "object" } },
                "steps": { "type": "array", "description": "[{tool, arguments, note?}]", "items": { "type": "object" } },
                "scope": { "type": "string", "enum": ["project", "user"], "description": "Default project; user: every project." }
            },
            "required": ["name", "description", "steps"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let name = file_safe(str_arg(args, "name").unwrap_or(""));
        let description = str_arg(args, "description").map(str::trim).unwrap_or("");
        let Some(raw_steps) = args["steps"].as_array().filter(|s| !s.is_empty()) else {
            return ToolResult::err("save_routine needs `name`, `description` and a non-empty `steps` list.");
        };
        if name.is_empty() || description.is_empty() {
            return ToolResult::err("save_routine needs `name`, `description` and a non-empty `steps` list.");
        }
        let mut steps = Vec::new();
        for raw in raw_steps {
            let Some(tool) = raw["tool"].as_str().filter(|t| !t.is_empty()) else {
                return ToolResult::err("Every step needs a `tool` name.");
            };
            if tool == RUN {
                return ToolResult::err("A routine cannot run another routine: put that routine's steps in instead.");
            }
            let mut step = Map::new();
            step.insert("tool".into(), json!(tool));
            step.insert("arguments".into(), if raw["arguments"].is_object() { raw["arguments"].clone() } else { json!({}) });
            if let Some(note) = raw["note"].as_str() {
                step.insert("note".into(), json!(note));
            }
            steps.push(Value::Object(step));
        }
        let inputs: Vec<Value> = args["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| {
                let n = i["name"].as_str().filter(|n| !n.is_empty())?;
                let mut input = json!({ "name": n, "description": i["description"].as_str().unwrap_or(""), "required": i["required"].as_bool().unwrap_or(true) });
                if let Some(d) = i["default"].as_str().or(i["defaultValue"].as_str()) {
                    input["defaultValue"] = json!(d);
                }
                Some(input)
            })
            .collect();
        let routine = json!({ "name": name, "description": description, "inputs": inputs, "steps": steps, "createdAt": iso_now() });
        let missing = undeclared(&routine);
        if !missing.is_empty() {
            return ToolResult::err(format!(
                "These placeholders have no matching input: {}. Declare them in `inputs` or remove the braces. Nothing was saved.",
                missing.join(", ")
            ));
        }
        let folder = match str_arg(args, "scope") {
            Some("user") => match &self.user {
                Some(user) => user.clone(),
                None => return ToolResult::err("There is no config folder for routines; save it with scope `project`."),
            },
            _ => self.project.clone(),
        };
        let file = folder.join(format!("{name}.json"));
        let replaced = file.exists();
        let written = std::fs::create_dir_all(&folder)
            .and_then(|_| std::fs::write(&file, serde_json::to_string_pretty(&routine).unwrap_or_default()));
        match written {
            Ok(()) => ToolResult::ok(format!(
                "Saved routine \"{name}\" ({} step{}) to {}{}. run_routine lists it from the next request.",
                steps.len(),
                if steps.len() == 1 { "" } else { "s" },
                file.display(),
                if replaced { ", replacing the earlier one" } else { "" }
            )),
            Err(e) => ToolResult::err(format!("Not saved: {e}.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    #[test]
    fn placeholders_fill_and_unknown_ones_stay() {
        let args = json!({ "command": "git tag v{version} && echo {other}", "list": ["{version}"], "n": 3 });
        let filled = fill(&args, &[("version".into(), "1.2.0".into())]);
        assert_eq!(filled, json!({ "command": "git tag v1.2.0 && echo {other}", "list": ["1.2.0"], "n": 3 }));
    }

    #[test]
    fn a_routine_must_declare_what_it_uses_and_get_what_it_needs() {
        let routine = json!({
            "name": "tag", "inputs": [{ "name": "version", "description": "the version", "required": true }, { "name": "msg", "required": false, "defaultValue": "release" }],
            "steps": [{ "tool": "run_shell", "arguments": { "command": "git tag -m '{msg}' v{version} {extra}", "json": "{\"a\": 1}" } }]
        });
        assert_eq!(undeclared(&routine), ["extra"], "JSON in a string is not a placeholder");
        assert_eq!(
            inputs_for(&routine, &json!({ "version": "2" })).unwrap(),
            [("version".to_string(), "2".to_string()), ("msg".into(), "release".into())]
        );
        assert_eq!(inputs_for(&routine, &json!({})).unwrap_err(), ["version (the version)"]);
        assert_eq!(file_safe(" Note to Word! "), "Note-to-Word");
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_794_576_600), "2026-11-13T13:30:00Z");
    }

    #[test]
    fn saved_routines_are_listed_and_the_project_wins_a_name() {
        let dir = temp_dir("routines");
        let (project, user) = (dir.join("p"), dir.join("u"));
        let save = SaveRoutine { project: project.clone(), user: Some(user.clone()) };
        let steps = json!([{ "tool": "run_shell", "arguments": { "command": "make {target}" } }]);
        let r = save.run(&json!({ "name": "build it", "description": "Build", "steps": steps, "inputs": [{ "name": "target" }] }));
        assert!(r.ok && r.text.contains("build-it"), "{}", r.text);
        assert!(!save.run(&json!({ "name": "x", "description": "d", "steps": steps })).ok, "undeclared {{target}}");
        assert!(!save.run(&json!({ "name": "x", "description": "d", "steps": [{ "tool": "run_routine" }] })).ok);
        save.run(&json!({ "name": "build it", "description": "User one", "steps": [{ "tool": "list_dir" }], "scope": "user" }));
        save.run(&json!({ "name": "lint", "description": "Lint", "steps": [{ "tool": "list_dir" }], "scope": "user" }));
        let listed = all(&[project.clone(), user]);
        let names: Vec<&str> = listed.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["build-it", "lint"]);
        assert_eq!(listed[0]["description"], "Build", "the project's wins");
        let definition = run_definition(&listed);
        assert!(definition["function"]["description"].as_str().unwrap().contains("• build-it — Build [target]"));
        mark_succeeded(std::slice::from_ref(&project), "build-it");
        assert!(all(&[project])[0]["lastSucceededAt"].as_str().unwrap().ends_with('Z'));
    }
}
