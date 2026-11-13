//! Memory: short notes the agent keeps for later sessions — how the person
//! likes things done, a lasting decision, a convention or path in the project.
//! From SwiftAgentKit's `remember` and `FileAgentMemoryStore`, made smaller:
//!
//! - two plain Markdown files, one note per line: `rak/memory.md` for this
//!   project (beside its other notes, see notes.rs; `.rak/memory.md` from
//!   alpha.6 is still read) and `~/.config/rak/memory.md` for every project.
//!   Edit or delete them freely;
//! - the notes are put in the system prompt when a session starts (not
//!   during it, so the prompt stays the same from call to call);
//! - `remember` asks first, like `save_skill`: a note steers every later
//!   session, so it must not get there because a web page or a file said so.
//!   SwiftAgentKit files notes with a separate model step; here the person's
//!   yes is the filter.

use crate::tools::{str_arg, Tool, ToolResult};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const FILE: &str = "memory.md";
/// Longest note, in characters.
const MAX_NOTE: usize = 500;
/// Most of one file put in the prompt; past it, the file asks to be tidied.
const MAX_FILE: usize = 6_000;

/// The project's memory file and the one for every project.
pub fn files(project: &Path, config_dir: Option<&Path>) -> (PathBuf, Option<PathBuf>) {
    (project.join(crate::notes::FOLDER).join(FILE), config_dir.map(|d| d.join(FILE)))
}

/// Where alpha.6 kept a project's notes: still read, never written.
pub fn legacy_file(project: &Path) -> PathBuf {
    project.join(".rak").join(FILE)
}

/// Pure — unit-tested. The notes in a memory file: its `- ` lines.
pub fn notes(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("- ")).map(str::trim).filter(|n| !n.is_empty()).map(String::from).collect()
}

/// Pure — unit-tested. The system prompt's block for both files' notes;
/// empty when there are none.
pub fn prompt_block(project: &[String], everywhere: &[String]) -> String {
    let fit = |list: &[String]| {
        let mut out = Vec::new();
        let mut size = 0;
        for note in list.iter().rev() {
            size += note.len() + 3;
            if size > MAX_FILE {
                break;
            }
            out.push(format!("- {note}"));
        }
        out.reverse();
        out
    };
    let mut block = String::new();
    for (title, list) in [("About the person and every project", everywhere), ("About this project", project)] {
        if !list.is_empty() {
            block.push_str(&format!("\n{title}:\n{}\n", fit(list).join("\n")));
        }
    }
    if block.is_empty() {
        return block;
    }
    format!(
        "\n\nNotes kept from earlier sessions (memory). They may be out of date: check before relying on one, and \
         say so when one is wrong.\n{}",
        block.trim_end()
    )
}

/// Reads both files' notes: (project, everywhere).
pub fn load(project_file: &Path, user_file: Option<&Path>) -> (Vec<String>, Vec<String>) {
    let read = |p: &Path| std::fs::read_to_string(p).map(|t| notes(&t)).unwrap_or_default();
    let mut project = read(project_file);
    // Notes from the old place come first (they are older), without repeats.
    if let Some(dir) = project_file.parent().and_then(Path::parent) {
        let mut old = read(&legacy_file(dir));
        old.retain(|n| !project.iter().any(|p| p.eq_ignore_ascii_case(n)));
        old.append(&mut project);
        project = old;
    }
    (project, user_file.map(read).unwrap_or_default())
}

/// Adds one note to a memory file. Ok(false): it was there already.
pub fn add(file: &Path, note: &str) -> Result<bool, String> {
    let note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    if notes(&existing).iter().any(|n| n.eq_ignore_ascii_case(&note)) {
        return Ok(false);
    }
    if existing.len() + note.len() > MAX_FILE {
        return Err(format!(
            "{} is full ({} characters). Ask the person to tidy it: remove notes that no longer hold",
            file.display(),
            existing.len()
        ));
    }
    let mut text = if existing.is_empty() {
        "# rak memory\n\nNotes rak keeps for later sessions, one per line. Edit or remove any.\n\n".to_string()
    } else {
        existing
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("- {note}\n"));
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    }
    std::fs::write(file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    Ok(true)
}

pub struct Remember {
    pub project_file: PathBuf,
    pub user_file: Option<PathBuf>,
}

impl Tool for Remember {
    fn name(&self) -> &str {
        "remember"
    }
    fn description(&self) -> &str {
        "Keep a note for later sessions: how the person likes things done, or a lasting decision or convention \
         here. One or two sentences; not details of this task. Asks first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note": { "type": "string" },
                "scope": { "type": "string", "enum": ["project", "everywhere"], "description": "Default project." }
            },
            "required": ["note"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(note) = str_arg(args, "note").map(str::trim).filter(|n| !n.is_empty()) else {
            return ToolResult::err("remember requires a `note`.");
        };
        if note.chars().count() > MAX_NOTE {
            return ToolResult::err(format!(
                "The note is {} characters; the most is {MAX_NOTE}. Say it in one or two sentences.",
                note.chars().count()
            ));
        }
        let file = match str_arg(args, "scope") {
            Some("everywhere") => match &self.user_file {
                Some(file) => file.clone(),
                None => return ToolResult::err("There is no config folder for notes about every project; use scope \"project\"."),
            },
            _ => self.project_file.clone(),
        };
        match add(&file, note) {
            Ok(true) => ToolResult::ok(format!("Kept in {}; later sessions will have it.", file.display())),
            Ok(false) => ToolResult::ok(format!("{} already has this note.", file.display())),
            Err(why) => ToolResult::err(format!("Not kept: {why}.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    #[test]
    fn notes_are_lines_and_go_in_the_prompt_newest_kept() {
        assert_eq!(notes("# rak memory\n\nintro\n\n- one\n-  two \n- \n* not this"), ["one", "two"]);
        assert_eq!(prompt_block(&[], &[]), "");
        let block = prompt_block(&["Tests run with make test.".into()], &["Prefers short answers.".into()]);
        assert!(block.contains("may be out of date"), "{block}");
        assert!(
            block.ends_with(
                "About the person and every project:\n- Prefers short answers.\n\nAbout this project:\n- Tests run with make test."
            ),
            "{block}"
        );
        let many: Vec<String> = (0..400).map(|i| format!("note number {i}")).collect();
        let block = prompt_block(&many, &[]);
        assert!(block.len() < MAX_FILE + 300 && block.ends_with("- note number 399"), "the newest kept");
    }

    #[test]
    fn remember_adds_once_and_stops_when_full() {
        let dir = temp_dir("memory");
        let tool = Remember { project_file: dir.join("rak").join(FILE), user_file: None };
        let first = tool.run(&json!({ "note": "Use  pnpm,\nnot npm." }));
        assert!(first.ok, "{}", first.text);
        assert!(tool.run(&json!({ "note": "use pnpm, not npm." })).text.contains("already has"));
        let text = std::fs::read_to_string(dir.join("rak/memory.md")).unwrap();
        assert!(text.starts_with("# rak memory") && text.ends_with("- Use pnpm, not npm.\n"), "{text}");
        assert_eq!(load(&dir.join("rak/memory.md"), None).0, ["Use pnpm, not npm."]);
        std::fs::create_dir_all(dir.join(".rak")).unwrap();
        std::fs::write(dir.join(".rak/memory.md"), "- An old note.\n- use pnpm, not npm.\n").unwrap();
        assert_eq!(load(&dir.join("rak/memory.md"), None).0, ["An old note.", "Use pnpm, not npm."], "alpha.6's file still read");
        assert!(!tool.run(&json!({ "note": "x", "scope": "everywhere" })).ok, "no config folder");
        assert!(!tool.run(&json!({ "note": "y".repeat(MAX_NOTE + 1) })).ok);
        for i in 0..200 {
            if !tool.run(&json!({ "note": format!("a fairly ordinary note about the project, number {i}") })).ok {
                assert!(i > 50, "full only after many notes");
                return;
            }
        }
        panic!("never full");
    }
}
