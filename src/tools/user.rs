//! The user's own tools: one small script per tool, in `~/.config/rak/tools`
//! (every project) or `./.rak/tools` (one project). rak reads each file's
//! header and offers the tool to the model like its own; a call runs the
//! script with the arguments as JSON on its input and returns what it
//! prints. Nothing runs until a tool is called, a script is only its own
//! logic, and a broken one breaks only itself.
//!
//! The header is comment lines at the top of the file:
//!
//! ```text
//! # rak-tool: csv_to_json
//! # description: Convert a CSV file to JSON.
//! # param: path (string, required) The CSV file.
//! # param: indent (integer) Spaces of indentation.
//! # needs: BRAVE_API_KEY
//! # safe: yes
//! # timeout: 30
//! ```
//!
//! `needs` names keys from rak's key file: the script gets those in its
//! environment and no others. `safe: yes` marks a tool that only reads, so it
//! runs without asking; any other asks first, like an edit or a command.

use super::{str_arg, Tool, ToolResult};
use crate::interrupt::Stop;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Seconds one call may take when the header gives no `timeout`.
pub const DEFAULT_TIMEOUT: u64 = 120;
/// The most a call's output hands back.
const MAX_OUTPUT: usize = 20_000;
/// The most a tool's script may be.
pub const MAX_SCRIPT: usize = 60_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    /// string, integer, number or boolean.
    pub kind: String,
    pub required: bool,
    pub description: String,
}

/// One tool, as its file's header describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    /// Keys from rak's key file the script is given.
    pub needs: Vec<String>,
    /// Only reads: runs without asking.
    pub safe: bool,
    pub timeout: u64,
    pub file: PathBuf,
}

/// Tools saved during a request, waiting to join the chat: the session
/// takes them when the request ends.
pub type Fresh = std::sync::Arc<std::sync::Mutex<Vec<Spec>>>;

/// Where a key's value comes from (rak's key file, then the environment).
pub type KeyOf = fn(&str) -> Option<String>;

/// Pure — unit-tested. A tool's name: lower-case letters, digits and `_`,
/// starting with a letter, at most 48 characters.
pub fn valid_name(name: &str) -> bool {
    name.len() <= 48
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Pure — unit-tested. A key's name: capitals, digits and `_`.
pub fn valid_key(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

const KINDS: [&str; 4] = ["string", "integer", "number", "boolean"];

/// Pure — unit-tested. `name (type, required) description` → the parameter.
fn parse_param(text: &str) -> Result<Param, String> {
    let text = text.trim();
    let (name, rest) = text.split_once('(').ok_or_else(|| format!("`param: {text}` needs a type: `param: name (string) what it is`"))?;
    let (inside, description) = rest.split_once(')').ok_or_else(|| format!("`param: {text}` is missing its `)`"))?;
    let name = name.trim();
    if !valid_name(name) {
        return Err(format!("a parameter's name is lower-case letters, digits and _, not `{name}`"));
    }
    let mut words = inside.split(',').map(str::trim);
    let kind = words.next().unwrap_or("").to_lowercase();
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("parameter `{name}` has the type `{kind}`; the types are {}", KINDS.join(", ")));
    }
    let required = words.any(|w| w.eq_ignore_ascii_case("required"));
    Ok(Param { name: name.to_string(), kind, required, description: description.trim().to_string() })
}

/// Pure — unit-tested. A script's header → the tool, or what is wrong with
/// it. Header lines are the comment lines (`#` or `//`) before the first
/// line of code; a `#!` line is passed over.
pub fn parse(text: &str, file: &Path) -> Result<Spec, String> {
    let mut spec = Spec {
        name: String::new(),
        description: String::new(),
        params: Vec::new(),
        needs: Vec::new(),
        safe: false,
        timeout: DEFAULT_TIMEOUT,
        file: file.to_path_buf(),
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("#!") {
            continue;
        }
        let Some(comment) = line.strip_prefix('#').or_else(|| line.strip_prefix("//")) else { break };
        let Some((key, value)) = comment.split_once(':') else { continue };
        let value = value.trim();
        match key.trim().to_lowercase().as_str() {
            "rak-tool" => spec.name = value.to_string(),
            "description" => spec.description = value.to_string(),
            "param" => spec.params.push(parse_param(value)?),
            "needs" => spec.needs.extend(value.split([',', ' ']).map(str::trim).filter(|k| !k.is_empty()).map(str::to_string)),
            "safe" => spec.safe = matches!(value.to_lowercase().as_str(), "yes" | "true"),
            "timeout" => {
                spec.timeout =
                    value.parse::<u64>().ok().filter(|s| (1..=3600).contains(s)).ok_or("`timeout` is a number of seconds, 1 to 3600")?
            }
            _ => {}
        }
    }
    if spec.name.is_empty() {
        return Err("no `# rak-tool: NAME` line at the top".to_string());
    }
    if !valid_name(&spec.name) {
        return Err(format!("a tool's name is lower-case letters, digits and _, starting with a letter, not `{}`", spec.name));
    }
    if spec.description.is_empty() {
        return Err("no `# description:` line".to_string());
    }
    if let Some(bad) = spec.needs.iter().find(|k| !valid_key(k)) {
        return Err(format!("`needs` names keys in capitals (BRAVE_API_KEY), not `{bad}`"));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(twice) = spec.params.iter().find(|p| !seen.insert(p.name.as_str())) {
        return Err(format!("parameter `{}` is listed twice", twice.name));
    }
    Ok(spec)
}

/// The languages a tool may be written in: its name, the file's ending, the
/// comment mark, and the programs that run it (the first one installed is used).
pub const LANGUAGES: [(&str, &str, &str, &[&str]); 3] =
    [("python", "py", "#", &["python3", "python"]), ("shell", "sh", "#", &["sh"]), ("node", "js", "//", &["node"])];

/// Pure — unit-tested. A tool's whole file: the header, then the code.
pub fn file_text(spec: &Spec, language: &str, code: &str) -> Result<String, String> {
    let (_, _, mark, programs) = LANGUAGES
        .iter()
        .find(|(name, ..)| *name == language)
        .ok_or_else(|| format!("the languages are python, shell and node, not `{language}`"))?;
    let mut lines = vec![format!("#!/usr/bin/env {}", programs[0]), format!("{mark} rak-tool: {}", spec.name)];
    lines.push(format!("{mark} description: {}", one_line(&spec.description)));
    for p in &spec.params {
        let required = if p.required { ", required" } else { "" };
        lines.push(format!("{mark} param: {} ({}{required}) {}", p.name, p.kind, one_line(&p.description)));
    }
    if !spec.needs.is_empty() {
        lines.push(format!("{mark} needs: {}", spec.needs.join(", ")));
    }
    if spec.safe {
        lines.push(format!("{mark} safe: yes"));
    }
    if spec.timeout != DEFAULT_TIMEOUT {
        lines.push(format!("{mark} timeout: {}", spec.timeout));
    }
    // The script's own "#!" line would end the header early if kept first.
    let code = code.trim_start_matches('\u{feff}');
    let code = if code.starts_with("#!") { code.split_once('\n').map(|(_, rest)| rest).unwrap_or("") } else { code };
    Ok(format!("{}\n\n{}\n", lines.join("\n"), code.trim()))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pure — unit-tested. The arguments' JSON Schema, for the model.
pub fn schema(spec: &Spec) -> Value {
    let mut properties = Map::new();
    for p in &spec.params {
        properties.insert(p.name.clone(), json!({ "type": p.kind, "description": p.description }));
    }
    let required: Vec<&str> = spec.params.iter().filter(|p| p.required).map(|p| p.name.as_str()).collect();
    json!({ "type": "object", "properties": properties, "required": required })
}

/// The tools in a folder, in name order, and one line for each file that is
/// not a tool or is broken. A folder that isn't there has none.
pub fn load_folder(folder: &Path) -> (Vec<Spec>, Vec<String>) {
    let (mut tools, mut problems) = (Vec::new(), Vec::new());
    let Ok(entries) = std::fs::read_dir(folder) else { return (tools, problems) };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.sort();
    for file in files {
        let ending = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !LANGUAGES.iter().any(|(_, known, ..)| *known == ending) {
            continue;
        }
        match std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|text| parse(&text, &file)) {
            Ok(spec) => tools.push(spec),
            Err(why) => problems.push(format!("{}: not loaded as a tool — {why}", file.display())),
        }
    }
    (tools, problems)
}

/// The tools from these folders; a name in an earlier folder wins (the
/// project's over the user's).
pub fn load(folders: &[PathBuf]) -> (Vec<Spec>, Vec<String>) {
    let (mut tools, mut problems): (Vec<Spec>, Vec<String>) = (Vec::new(), Vec::new());
    for folder in folders {
        let (found, mut wrong) = load_folder(folder);
        problems.append(&mut wrong);
        for spec in found {
            if !tools.iter().any(|t| t.name == spec.name) {
                tools.push(spec);
            }
        }
    }
    (tools, problems)
}

/// Where tools are kept, the project's folder first.
pub fn folders(project: &Path, config_dir: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    (project.join(".rak").join("tools"), config_dir.map(|dir| dir.join("tools")))
}

/// Deletes the tool named `name` from `folder`. Returns the file removed.
pub fn remove(folder: &Path, name: &str) -> Result<PathBuf, String> {
    let (tools, _) = load_folder(folder);
    let spec = tools.into_iter().find(|t| t.name == name).ok_or_else(|| format!("no tool named {name} in {}", folder.display()))?;
    std::fs::remove_file(&spec.file).map_err(|e| format!("cannot remove {}: {e}", spec.file.display()))?;
    Ok(spec.file)
}

/// The program that runs this file: the first installed one for its ending.
fn runner(file: &Path) -> Result<String, String> {
    let ending = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let (language, _, _, programs) = LANGUAGES
        .iter()
        .find(|(_, known, ..)| *known == ending)
        .ok_or_else(|| format!("{} is not a python, shell or node file", file.display()))?;
    programs
        .iter()
        .find(|program| installed(program))
        .map(|program| program.to_string())
        .ok_or_else(|| format!("this tool is written in {language}, and {} is not installed on this machine", programs.join(" or ")))
}

/// Whether a program is on the PATH.
pub fn installed(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|folder| {
        let plain = folder.join(program);
        plain.is_file() || (cfg!(windows) && ["exe", "cmd", "bat"].iter().any(|ending| plain.with_extension(ending).is_file()))
    })
}

/// The languages installed here, for save_tool to say when one is missing.
pub fn languages_here() -> Vec<&'static str> {
    LANGUAGES.iter().filter(|(_, _, _, programs)| programs.iter().any(|p| installed(p))).map(|(name, ..)| *name).collect()
}

/// Pure — unit-tested. Whether an environment variable's name reads as a
/// secret: a tool's script is not handed these unless its header asks.
pub fn reads_as_secret(name: &str) -> bool {
    let upper = name.to_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL"].iter().any(|word| upper.contains(word))
}

/// Runs the tool's script once: the arguments as JSON on its input, its
/// declared keys in its environment, and what it prints as the result.
pub fn run_script(spec: &Spec, args: &Value, cwd: &Path, key_of: KeyOf, stop: &Stop) -> ToolResult {
    let mut keys = Vec::new();
    for name in &spec.needs {
        match key_of(name) {
            Some(value) => keys.push((name.clone(), value)),
            None => {
                return ToolResult::err(format!(
                    "The tool {} needs the key {name}, which is not set. Stop and tell the user to type /key {name} (it is typed hidden and kept in rak's key file); do not ask them to paste the key in the chat.",
                    spec.name
                ))
            }
        }
    }
    let program = match runner(&spec.file) {
        Ok(program) => program,
        Err(why) => return ToolResult::err(why),
    };
    let mut command = Command::new(&program);
    command.arg(&spec.file).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Only the keys the header names: other secrets in rak's environment stay out.
    for (name, _) in std::env::vars_os().filter_map(|(name, _)| name.into_string().ok().map(|name| (name, ()))) {
        if reads_as_secret(&name) {
            command.env_remove(&name);
        }
    }
    command.envs(keys.iter().map(|(name, value)| (name.as_str(), value.as_str())));
    // Its own group, so a stop or a timeout ends whatever it started too.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return ToolResult::err(format!("couldn't start {program} for the tool {}: {e}", spec.name)),
    };
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(args.to_string().as_bytes());
    }
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.by_ref().take(4 * MAX_OUTPUT as u64).read_to_end(&mut text);
                // Past the limit the rest is read and dropped, so the script is not left blocked on a full pipe.
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
            String::from_utf8_lossy(&text).to_string()
        })
    };
    let out = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => break Err(format!("waiting for the tool failed: {e}")),
        }
        if stop.requested() {
            super::shell::kill_tree(&mut child);
            let _ = child.wait();
            break Err("stopped by the user".to_string());
        }
        if started.elapsed() > Duration::from_secs(spec.timeout) {
            super::shell::kill_tree(&mut child);
            let _ = child.wait();
            break Err(format!("no result within {} s; the tool was stopped", spec.timeout));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (printed, said) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    // A key that the script printed never reaches the model.
    let hide = |text: String| {
        keys.iter()
            .filter(|(_, value)| value.len() >= 6)
            .fold(text, |text, (name, value)| text.replace(value.as_str(), &format!("[{name}]")))
    };
    let (printed, said) = (hide(printed), hide(said));
    match status {
        Err(why) => ToolResult::err(format!("{}: {why}.{}", spec.name, tail_note(&said))),
        Ok(status) if status.success() => {
            let text = super::shell::head_and_tail(printed.trim_end(), MAX_OUTPUT);
            ToolResult::ok(if text.is_empty() { "(the tool printed nothing)".to_string() } else { text })
        }
        Ok(status) => {
            let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".to_string());
            let shown = super::shell::head_and_tail(printed.trim_end(), MAX_OUTPUT / 2);
            ToolResult::err(format!(
                "{} ended with {code}.{}{}",
                spec.name,
                tail_note(&said),
                if shown.is_empty() { String::new() } else { format!("\nIt printed:\n{shown}") }
            ))
        }
    }
}

fn tail_note(said: &str) -> String {
    let said = said.trim();
    if said.is_empty() {
        String::new()
    } else {
        format!("\nIt said:\n{}", super::shell::head_and_tail(said, 4000))
    }
}

/// One of the user's tools, offered to the model.
pub struct UserTool {
    pub spec: Spec,
    pub cwd: PathBuf,
    pub key_of: KeyOf,
    pub stop: Stop,
}

impl Tool for UserTool {
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn parameters(&self) -> Value {
        schema(&self.spec)
    }
    fn needs_approval(&self) -> bool {
        !self.spec.safe
    }
    fn run(&self, args: &Value) -> ToolResult {
        run_script(&self.spec, args, &self.cwd, self.key_of, &self.stop)
    }
}

/// save_tool: writes a tool's file after the user has seen it, tries it once
/// when a test input comes with it, and queues it to join the chat.
pub struct SaveTool {
    pub project: PathBuf,
    pub user: Option<PathBuf>,
    pub cwd: PathBuf,
    pub key_of: KeyOf,
    pub stop: Stop,
    pub fresh: Fresh,
    /// Names rak's own tools and the MCP servers' tools have: not to be taken.
    pub taken: Vec<String>,
}

/// Pure — unit-tested. The tool save_tool was asked to save, from its arguments.
pub fn spec_from(args: &Value, file: &Path) -> Result<Spec, String> {
    let field = |key: &str| str_arg(args, key).map(str::trim).filter(|v| !v.is_empty());
    let (Some(name), Some(description)) = (field("name"), field("description")) else {
        return Err("save_tool requires `name`, `description`, `language` and `code`".to_string());
    };
    if !valid_name(name) {
        return Err(format!("a tool's name is lower-case letters, digits and _, starting with a letter (csv_to_json), not `{name}`"));
    }
    let mut params = Vec::new();
    for entry in args["params"].as_array().into_iter().flatten() {
        let text = |key: &str| entry[key].as_str().unwrap_or("").trim().to_string();
        let kind = if text("type").is_empty() { "string".to_string() } else { text("type").to_lowercase() };
        if !valid_name(&text("name")) {
            return Err(format!("a parameter's name is lower-case letters, digits and _, not `{}`", text("name")));
        }
        if !KINDS.contains(&kind.as_str()) {
            return Err(format!("parameter `{}` has the type `{kind}`; the types are {}", text("name"), KINDS.join(", ")));
        }
        params.push(Param { name: text("name"), kind, required: entry["required"] == true, description: text("description") });
    }
    let needs: Vec<String> =
        args["needs"].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(|k| k.trim().to_string())).collect();
    if let Some(bad) = needs.iter().find(|k| !valid_key(k)) {
        return Err(format!("`needs` names keys in capitals (BRAVE_API_KEY), not `{bad}`"));
    }
    let timeout = args["timeout"].as_u64().filter(|s| (1..=3600).contains(s)).unwrap_or(DEFAULT_TIMEOUT);
    Ok(Spec {
        name: name.to_string(),
        description: description.to_string(),
        params,
        needs,
        safe: args["safe"] == true,
        timeout,
        file: file.to_path_buf(),
    })
}

impl Tool for SaveTool {
    fn name(&self) -> &str {
        "save_tool"
    }
    fn description(&self) -> &str {
        "Save a tool of the user's own: one script rak runs when the tool is called (same name: replaces it). \
         Load the skill \"Make tool\" first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "lower_case_with_underscores" },
                "description": { "type": "string", "description": "One line the model reads to decide when to call it." },
                "language": { "type": "string", "enum": ["python", "shell", "node"] },
                "code": { "type": "string", "description": "The script: reads its arguments as one JSON object on standard input, prints the result." },
                "params": { "type": "array", "items": { "type": "object", "properties": {
                    "name": { "type": "string" }, "type": { "type": "string", "enum": KINDS },
                    "required": { "type": "boolean" }, "description": { "type": "string" } }, "required": ["name"] } },
                "needs": { "type": "array", "items": { "type": "string" }, "description": "Names of keys it reads from its environment (BRAVE_API_KEY)." },
                "safe": { "type": "boolean", "description": "true only if it changes nothing: then it runs without asking." },
                "timeout": { "type": "integer", "description": "Seconds one call may take (default 120)." },
                "scope": { "type": "string", "enum": ["user", "project"], "description": "user (default): every project." },
                "test_input": { "type": "object", "description": "Arguments to try it with once, right after saving." }
            },
            "required": ["name", "description", "language", "code"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let (Some(language), Some(code)) = (str_arg(args, "language"), str_arg(args, "code").filter(|c| !c.trim().is_empty())) else {
            return ToolResult::err("save_tool requires `name`, `description`, `language` and `code`.");
        };
        let Some((_, ending, _, programs)) = LANGUAGES.iter().find(|(name, ..)| *name == language) else {
            return ToolResult::err("The languages are python, shell and node.");
        };
        if !programs.iter().any(|p| installed(p)) {
            let here = languages_here();
            return ToolResult::err(format!(
                "{language} is not installed on this machine, so the tool could not run. Installed here: {}.",
                if here.is_empty() { "none of python, shell, node".to_string() } else { here.join(", ") }
            ));
        }
        if code.len() > MAX_SCRIPT {
            return ToolResult::err(format!("The script is over {} KB: a tool is a small script.", MAX_SCRIPT / 1000));
        }
        let folder = match str_arg(args, "scope") {
            Some("project") => self.project.clone(),
            _ => match &self.user {
                Some(user) => user.clone(),
                None => return ToolResult::err("There is no config folder for tools; save it with scope `project`."),
            },
        };
        let name = str_arg(args, "name").unwrap_or("").trim().to_string();
        let spec = match spec_from(args, &folder.join(format!("{name}.{ending}"))) {
            Ok(spec) => spec,
            Err(why) => return ToolResult::err(format!("Not saved: {why}.")),
        };
        if self.taken.contains(&spec.name) {
            return ToolResult::err(format!("Not saved: rak already has a tool named {}; pick another name.", spec.name));
        }
        let text = match file_text(&spec, language, code) {
            Ok(text) => text,
            Err(why) => return ToolResult::err(format!("Not saved: {why}.")),
        };
        // The same tool in another language is replaced, not kept beside it.
        let (existing, _) = load_folder(&folder);
        let replaced = existing.iter().any(|t| t.name == spec.name);
        for old in existing.iter().filter(|t| t.name == spec.name && t.file != spec.file) {
            let _ = std::fs::remove_file(&old.file);
        }
        if let Err(e) = std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&spec.file, &text)) {
            return ToolResult::err(format!("Not saved: cannot write {}: {e}.", spec.file.display()));
        }
        let mut said = format!(
            "Saved the tool {} to {}.{} It joins this chat with the user's next message; you cannot call it in this reply.",
            spec.name,
            spec.file.display(),
            if replaced { " It replaces the earlier version." } else { "" }
        );
        if let Some(input) = args.get("test_input").filter(|v| v.is_object()) {
            let tried = run_script(&spec, input, &self.cwd, self.key_of, &self.stop);
            said.push_str(&format!(
                "\n\nTried once with {input}: {}\n{}",
                if tried.ok { "it worked." } else { "it failed. Fix the script and save it again under the same name." },
                tried.text
            ));
        }
        let mut fresh = self.fresh.lock().unwrap();
        fresh.retain(|t| t.name != spec.name);
        fresh.push(spec);
        ToolResult::ok(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    const SCRIPT: &str = "#!/usr/bin/env python3\n# rak-tool: csv_to_json\n# description: Convert a CSV file to JSON.\n# param: path (string, required) The CSV file.\n# param: indent (integer) Spaces of indentation.\n# needs: BRAVE_API_KEY, OTHER_TOKEN\n# safe: yes\n# timeout: 30\n\nimport sys\n# rak-tool: not_this_one\nprint('x')\n";

    #[test]
    fn a_header_is_read_and_what_is_wrong_with_one_is_said() {
        let spec = parse(SCRIPT, Path::new("/t/csv_to_json.py")).unwrap();
        assert_eq!((spec.name.as_str(), spec.description.as_str()), ("csv_to_json", "Convert a CSV file to JSON."));
        assert_eq!(spec.params.len(), 2);
        assert_eq!(
            spec.params[0],
            Param { name: "path".into(), kind: "string".into(), required: true, description: "The CSV file.".into() }
        );
        assert!(!spec.params[1].required && spec.params[1].kind == "integer");
        assert_eq!(spec.needs, ["BRAVE_API_KEY", "OTHER_TOKEN"]);
        assert!(spec.safe && spec.timeout == 30, "a comment after the code starts is not the header");
        // A node file's header uses //, and what is left out has its defaults.
        let js = parse("// rak-tool: hello\n// description: Says hello.\nconsole.log('hi')\n", Path::new("/t/hello.js")).unwrap();
        assert!(!js.safe && js.timeout == DEFAULT_TIMEOUT && js.params.is_empty() && js.needs.is_empty());

        let wrong = |text: &str| parse(text, Path::new("/t/x.py")).unwrap_err();
        assert!(wrong("print('x')\n").contains("rak-tool"));
        assert!(wrong("# rak-tool: Bad-Name\n# description: d\n").contains("lower-case"));
        assert!(wrong("# rak-tool: ok\n").contains("description"));
        assert!(wrong("# rak-tool: ok\n# description: d\n# param: path The file\n").contains("needs a type"));
        assert!(wrong("# rak-tool: ok\n# description: d\n# param: path (file) x\n").contains("the types are"));
        assert!(wrong("# rak-tool: ok\n# description: d\n# param: a (string) x\n# param: a (string) y\n").contains("twice"));
        assert!(wrong("# rak-tool: ok\n# description: d\n# needs: brave key\n").contains("capitals"));
        assert!(wrong("# rak-tool: ok\n# description: d\n# timeout: soon\n").contains("seconds"));
    }

    #[test]
    fn a_saved_file_reads_back_as_the_same_tool() {
        let args = json!({ "name": "word_count", "description": "Counts  the words\nin a text.", "language": "python",
            "code": "#!/usr/bin/python\nimport sys, json\nprint(len(json.load(sys.stdin)['text'].split()))",
            "params": [{ "name": "text", "type": "string", "required": true, "description": "The text." }, { "name": "unique" , "type": "boolean" }],
            "needs": ["SOME_API_KEY"], "safe": true, "timeout": 10 });
        let file = Path::new("/t/word_count.py");
        let spec = spec_from(&args, file).unwrap();
        let text = file_text(&spec, "python", args["code"].as_str().unwrap()).unwrap();
        assert!(text.starts_with("#!/usr/bin/env python3\n# rak-tool: word_count\n# description: Counts the words in a text.\n"), "{text}");
        assert_eq!(text.matches("#!").count(), 1, "the script's own #! line is dropped: {text}");
        assert_eq!(parse(&text, file).unwrap(), Spec { description: "Counts the words in a text.".into(), ..spec.clone() });
        let schema = schema(&spec);
        assert_eq!(schema["properties"]["text"], json!({ "type": "string", "description": "The text." }));
        assert_eq!(schema["required"], json!(["text"]));
        // A node tool's header is // comments.
        assert!(file_text(&spec, "node", "console.log(1)").unwrap().contains("// rak-tool: word_count"));
        assert!(file_text(&spec, "ruby", "puts 1").unwrap_err().contains("python, shell and node"));
        for (bad, says) in [
            (json!({ "name": "Word Count", "description": "d" }), "lower-case"),
            (json!({ "description": "d" }), "requires"),
            (json!({ "name": "ok", "description": "d", "needs": ["lower"] }), "capitals"),
            (json!({ "name": "ok", "description": "d", "params": [{ "name": "a", "type": "list" }] }), "the types are"),
        ] {
            assert!(spec_from(&bad, file).unwrap_err().contains(says), "{bad}");
        }
    }

    #[test]
    fn names_and_secrets_are_told_apart() {
        assert!(
            valid_name("csv_to_json")
                && valid_name("a1")
                && !valid_name("1a")
                && !valid_name("Csv")
                && !valid_name("a-b")
                && !valid_name("")
        );
        assert!(valid_key("BRAVE_API_KEY") && !valid_key("brave") && !valid_key("A KEY") && !valid_key(""));
        for name in ["OPENAI_API_KEY", "GITHUB_TOKEN", "aws_secret_access_key", "DB_PASSWORD"] {
            assert!(reads_as_secret(name), "{name}");
        }
        for name in ["PATH", "HOME", "LANG", "EDITOR"] {
            assert!(!reads_as_secret(name), "{name}");
        }
    }

    #[cfg(unix)]
    fn shell_tool(dir: &Path, name: &str, header: &str, body: &str) -> Spec {
        let file = dir.join(format!("{name}.sh"));
        std::fs::write(&file, format!("# rak-tool: {name}\n# description: A test tool.\n{header}\n{body}\n")).unwrap();
        parse(&std::fs::read_to_string(&file).unwrap(), &file).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_runs_with_its_arguments_and_only_its_own_keys() {
        let dir = temp_dir("user-tool-run");
        fn keys(name: &str) -> Option<String> {
            (name == "MY_TEST_KEY").then(|| "sekret-value-123".to_string())
        }
        // The arguments arrive as JSON on its input; what it prints is the result.
        let echo = shell_tool(&dir, "echo_input", "# param: text (string, required) x", "cat");
        let result = run_script(&echo, &json!({ "text": "hello" }), &dir, keys, &Stop::new());
        assert!(result.ok && result.text == r#"{"text":"hello"}"#, "{}", result.text);

        // A declared key is in its environment, and is hidden if the script prints it.
        let shows = shell_tool(&dir, "shows_key", "# needs: MY_TEST_KEY", "echo \"key=$MY_TEST_KEY other=${UNDECLARED_TEST_TOKEN:-none}\"");
        std::env::set_var("UNDECLARED_TEST_TOKEN", "leaky");
        let result = run_script(&shows, &json!({}), &dir, keys, &Stop::new());
        std::env::remove_var("UNDECLARED_TEST_TOKEN");
        assert_eq!(result.text, "key=[MY_TEST_KEY] other=none", "its key is passed and masked; another secret is not passed");

        // A key that is not set: the tool does not run, and the user is sent to /key.
        let needs = shell_tool(&dir, "needs_key", "# needs: MISSING_KEY", "echo ran");
        let result = run_script(&needs, &json!({}), &dir, keys, &Stop::new());
        assert!(!result.ok && result.text.contains("/key MISSING_KEY") && !result.text.contains("ran"), "{}", result.text);

        // A failure says the exit code and what the script said; a slow one is stopped.
        let fails = shell_tool(&dir, "fails", "", "echo partial; echo 'bad input' >&2; exit 3");
        let result = run_script(&fails, &json!({}), &dir, keys, &Stop::new());
        assert!(
            !result.ok && result.text.contains("ended with 3") && result.text.contains("bad input") && result.text.contains("partial"),
            "{}",
            result.text
        );
        let slow = shell_tool(&dir, "slow", "# timeout: 1", "sleep 20");
        let began = Instant::now();
        let result = run_script(&slow, &json!({}), &dir, keys, &Stop::new());
        assert!(!result.ok && result.text.contains("no result within 1 s") && began.elapsed() < Duration::from_secs(8), "{}", result.text);
        let nothing = shell_tool(&dir, "quiet", "", "true");
        assert_eq!(run_script(&nothing, &json!({}), &dir, keys, &Stop::new()).text, "(the tool printed nothing)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn save_tool_writes_the_file_tries_it_and_queues_it_and_folders_are_read() {
        let dir = temp_dir("user-tool-save");
        fn no_keys(_: &str) -> Option<String> {
            None
        }
        let (project, user) = (dir.join("project"), dir.join("user"));
        let fresh: Fresh = Default::default();
        let save = SaveTool {
            project: project.clone(),
            user: Some(user.clone()),
            cwd: dir.clone(),
            key_of: no_keys,
            stop: Stop::new(),
            fresh: fresh.clone(),
            taken: vec!["read_file".into()],
        };
        let args = json!({ "name": "shout", "description": "Upper-cases its text.", "language": "shell", "code": "tr a-z A-Z",
            "params": [{ "name": "text", "type": "string", "required": true }], "test_input": { "text": "hi" } });
        let result = save.run(&args);
        assert!(
            result.ok
                && result.text.contains("Saved the tool shout")
                && result.text.contains("it worked")
                && result.text.contains(r#"{"TEXT":"HI"}"#),
            "{}",
            result.text
        );
        assert!(user.join("shout.sh").is_file());
        assert_eq!(fresh.lock().unwrap().len(), 1);
        // The same name again replaces it, in the queue too.
        let again = save.run(&json!({ "name": "shout", "description": "Louder.", "language": "shell", "code": "tr a-z A-Z" }));
        assert!(again.text.contains("replaces the earlier version"), "{}", again.text);
        assert_eq!(fresh.lock().unwrap().iter().map(|t| t.description.as_str()).collect::<Vec<_>>(), ["Louder."]);
        // A failing test is reported with what to do; a taken name and a missing field are refused.
        let broken = save.run(
            &json!({ "name": "broken", "description": "d", "language": "shell", "code": "exit 2", "test_input": {}, "scope": "project" }),
        );
        assert!(broken.ok && broken.text.contains("it failed") && broken.text.contains("ended with 2"), "{}", broken.text);
        assert!(project.join("broken.sh").is_file());
        assert!(save
            .run(&json!({ "name": "read_file", "description": "d", "language": "shell", "code": "true" }))
            .text
            .contains("already has a tool named"));
        assert!(!save.run(&json!({ "name": "x", "description": "d", "language": "shell" })).ok);

        // Folders: the project's tool wins over the user's of the same name; a broken file is a line, not a stop.
        std::fs::write(project.join("shout.sh"), "# rak-tool: shout\n# description: The project's.\ncat\n").unwrap();
        std::fs::write(user.join("notes.txt"), "not a tool").unwrap();
        std::fs::write(user.join("bad.py"), "print('no header')\n").unwrap();
        let (tools, problems) = load(&[project.clone(), user.clone()]);
        let found: Vec<(&str, &str)> = tools.iter().map(|t| (t.name.as_str(), t.description.as_str())).collect();
        assert_eq!(found, [("broken", "d"), ("shout", "The project's.")]);
        assert!(problems.len() == 1 && problems[0].contains("bad.py") && problems[0].contains("rak-tool"), "{problems:?}");
        assert!(load_folder(&dir.join("nowhere")).0.is_empty());
        // Removing deletes the file.
        assert_eq!(remove(&user, "shout").unwrap(), user.join("shout.sh"));
        assert!(remove(&user, "shout").unwrap_err().contains("no tool named shout"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
