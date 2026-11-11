//! File tools. Descriptions and behaviour follow SwiftAgentKit's.

use super::{bool_arg, int_arg, str_arg, verify, Ctx, Tool, ToolResult};
use crate::read_slice::ReadSlice;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

fn read_text(path: &Path, raw: &str, verb: &str) -> Result<String, ToolResult> {
    let bytes = std::fs::read(path).map_err(|_| ToolResult::err(format!("Cannot read file{verb}: {raw}")))?;
    String::from_utf8(bytes).map_err(|_| ToolResult::err(format!("Not a UTF-8 text file: {raw}")))
}

// MARK: read_file

pub struct ReadFile(pub Arc<Ctx>);

impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read a UTF-8 text file and return its contents. Files up to ~40,000 characters come back whole — \
         read them in ONE call. Use `offset` and `limit` only to page through genuinely large files, in big \
         slices: every call re-sends the whole conversation, so many small reads cost far more than one large one. \
         In a big file, `start_line` and `end_line` (from code_map) read just that part: take a whole function or more."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path (a leading ~ is expanded)." },
                "offset": { "type": "integer", "description": "Start character offset (default 0)." },
                "limit": { "type": "integer", "description": "Max characters to read (default 40000). Small values are widened — paging in tiny slices is the most expensive way to read a file." },
                "start_line": { "type": "integer", "description": "First line to read (1-based); instead of offset." },
                "end_line": { "type": "integer", "description": "Last line to read (default: start_line + 200)." }
            },
            "required": ["path"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(raw) = str_arg(args, "path").filter(|s| !s.is_empty()) else {
            return ToolResult::err("read_file requires a `path`.");
        };
        let path = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&path) {
            return ToolResult::err(format!("Refusing to read {raw}: {reason}."));
        }
        let text = match read_text(&path, raw, "") {
            Ok(t) => t,
            Err(e) => return e,
        };
        if let Some(first) = int_arg(args, "start_line") {
            self.0.note_seen(&path);
            return ToolResult::ok(lines_of(&text, first.max(1) as usize, int_arg(args, "end_line").map(|n| n.max(1) as usize)));
        }
        let chars: Vec<char> = text.chars().collect();
        let to_usize = |key| int_arg(args, key).map(|n| n.max(0) as usize);
        let slice = ReadSlice::new(chars.len(), to_usize("offset"), to_usize("limit"));
        let body: String = chars[slice.start..slice.end].iter().collect();
        self.0.note_seen(&path);
        ToolResult::ok(slice.annotate(&body))
    }
}

/// Pure — unit-tested. Lines `first` to `last` (1-based, inclusive; at most
/// 2,000), with a header saying where they are in the file.
pub fn lines_of(text: &str, first: usize, last: Option<usize>) -> String {
    let total = text.lines().count();
    if first > total {
        return format!("[the file has {total} lines; line {first} is past its end]");
    }
    let last = last.unwrap_or(first + 200).clamp(first, total).min(first + 1_999);
    let body: Vec<&str> = text.lines().skip(first - 1).take(last - first + 1).collect();
    let more = if last < total { format!(" — call again with start_line {} for more", last + 1) } else { String::new() };
    format!("[lines {first}–{last} of {total}{more}]\n{}", body.join("\n"))
}

// MARK: write_file

pub struct WriteFile(pub Arc<Ctx>);

impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Write UTF-8 text to a file, creating it (and any missing parent folders) if needed. \
         Overwrites by default; set `append: true` to append. Requires approval."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path (a leading ~ is expanded)." },
                "content": { "type": "string", "description": "The text to write." },
                "append": { "type": "boolean", "description": "Append instead of overwrite (default false)." }
            },
            "required": ["path", "content"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(raw) = str_arg(args, "path").filter(|s| !s.is_empty()) else {
            return ToolResult::err("write_file requires a `path`.");
        };
        let Some(content) = str_arg(args, "content") else {
            return ToolResult::err("write_file requires `content`.");
        };
        let path = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&path) {
            return ToolResult::err(format!("Refusing to write {raw}: {reason}."));
        }
        let append = bool_arg(args, "append").unwrap_or(false);

        // Appending adds to whatever is there; overwriting a file that changed
        // since this run last saw it would throw that change away.
        if !append {
            if let Some(stale) = self.0.stale_reason(&path) {
                return ToolResult::err(format!(
                    "Not written: {raw} — {stale}. Read it again, then write a version that keeps those \
                     changes (or use edit_file for a targeted change)."
                ));
            }
        }
        // The file as it would be after this write must pass the check; if
        // not, nothing is written.
        let landed = if append && path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(existing) => Some(existing + content),
                Err(_) => None, // not text: appending is not judged
            }
        } else {
            Some(content.to_string())
        };
        if let Some(rejection) = landed.and_then(|text| verify::check(&path, &text)) {
            return ToolResult::err(rejection.message(raw));
        }
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolResult::err(format!("Write failed: {e}"));
            }
        }
        let written = if append && path.exists() {
            use std::io::Write;
            std::fs::OpenOptions::new().append(true).open(&path).and_then(|mut f| f.write_all(content.as_bytes()))
        } else {
            std::fs::write(&path, content)
        };
        if let Err(e) = written {
            return ToolResult::err(format!("Write failed: {e}"));
        }
        self.0.note_seen(&path);
        let verb = if append { "Appended" } else { "Wrote" };
        ToolResult::ok(format!("{verb} {} bytes to {raw}.", content.len()))
    }
}

// MARK: edit_file

pub struct EditFile(pub Arc<Ctx>);

impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }
    fn description(&self) -> &'static str {
        "Edit an existing file by replacing exact text: `old_text` → `new_text`. Preferred for changes to an \
         existing file: no diff markers needed. `old_text` must occur exactly once — include a few surrounding \
         lines to make it unique — or set `replace_all`. Trailing whitespace is tolerated. If it isn't found, \
         the error shows the file's current lines near the closest match. Requires approval."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to edit (a leading ~ is expanded)." },
                "old_text": { "type": "string", "description": "The exact text to replace, copied from the file." },
                "new_text": { "type": "string", "description": "The replacement text." },
                "replace_all": { "type": "boolean", "description": "Replace every occurrence (default false: exactly one)." }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(raw) = str_arg(args, "path").filter(|s| !s.is_empty()) else {
            return ToolResult::err("edit_file requires a `path`.");
        };
        let Some(old) = str_arg(args, "old_text").filter(|s| !s.is_empty()) else {
            return ToolResult::err("edit_file requires `old_text` (the exact text to replace).");
        };
        let Some(new) = str_arg(args, "new_text") else {
            return ToolResult::err("edit_file requires `new_text`.");
        };
        let replace_all = bool_arg(args, "replace_all").unwrap_or(false);
        let path = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&path) {
            return ToolResult::err(format!("Refusing to edit {raw}: {reason}."));
        }
        let source = match read_text(&path, raw, " to edit") {
            Ok(t) => t,
            Err(e) if e.text.starts_with("Cannot read") => {
                return ToolResult::err(format!("{}. Use write_file to create a new file.", e.text))
            }
            Err(e) => return e,
        };

        let too_many = |count: usize| {
            ToolResult::err(format!(
                "`old_text` occurs {count} times in {raw}. Include more surrounding lines to make it unique, \
                 or set replace_all to change every occurrence. Nothing was changed."
            ))
        };
        let exact = source.matches(old).count();
        let (edited, count) = if exact > 0 {
            if exact > 1 && !replace_all {
                return too_many(exact);
            }
            (if replace_all { source.replace(old, new) } else { source.replacen(old, new, 1) }, exact)
        } else {
            match replace_ignoring_trailing_whitespace(old, new, &source, replace_all) {
                Some((_, n)) if n > 1 && !replace_all => return too_many(n),
                Some(done) => done,
                None => {
                    return ToolResult::err(format!("`old_text` was not found in {raw}. Nothing was changed. {}", nearby(old, &source)))
                }
            }
        };
        if let Some(rejection) = verify::check(&path, &edited) {
            return ToolResult::err(rejection.message(raw));
        }
        if let Err(e) = std::fs::write(&path, edited) {
            return ToolResult::err(format!("Write failed: {e}"));
        }
        self.0.note_seen(&path);
        ToolResult::ok(format!("Replaced {count} occurrence{} in {raw}.", if count == 1 { "" } else { "s" }))
    }
}

/// Line-wise match with trailing spaces and tabs ignored. Returns the edited
/// text and how many blocks matched, or None when none did. With more than one
/// match and `all` false, nothing is replaced and only the count is useful.
pub fn replace_ignoring_trailing_whitespace(old: &str, new: &str, source: &str, all: bool) -> Option<(String, usize)> {
    fn rstrip(s: &str) -> &str {
        s.trim_end_matches([' ', '\t'])
    }
    let mut lines: Vec<&str> = source.split('\n').collect();
    let want: Vec<&str> = old.split('\n').map(rstrip).collect();
    if want.is_empty() || want.len() > lines.len() {
        return None;
    }
    let mut starts = Vec::new();
    let mut i = 0;
    while i + want.len() <= lines.len() {
        if (0..want.len()).all(|k| rstrip(lines[i + k]) == want[k]) {
            starts.push(i);
            i += want.len();
        } else {
            i += 1;
        }
    }
    if starts.is_empty() {
        return None;
    }
    if starts.len() > 1 && !all {
        return Some((String::new(), starts.len()));
    }
    let replacement: Vec<&str> = new.split('\n').collect();
    for &s in starts.iter().rev() {
        lines.splice(s..s + want.len(), replacement.iter().copied());
    }
    Some((lines.join("\n"), starts.len()))
}

/// The file's lines around the closest match of `old`'s first line.
pub fn nearby(old: &str, source: &str) -> String {
    let lines: Vec<&str> = source.split('\n').collect();
    let first = old.split('\n').find(|l| !l.is_empty()).map(str::trim).unwrap_or("");
    let prefix: String = first.chars().take(12).collect();
    let long = first.chars().count() > 12;
    let hit =
        (!first.is_empty()).then(|| lines.iter().position(|l| l.contains(first) || (long && l.trim().starts_with(&prefix)))).flatten();
    let Some(hit) = hit else {
        return "Read the file and copy the text exactly.".to_string();
    };
    let lo = hit.saturating_sub(4);
    let hi = (hit + 6).min(lines.len() - 1);
    let shown: Vec<String> = (lo..=hi).map(|n| format!("{} | {}", n + 1, lines[n])).collect();
    format!("The file's CURRENT lines near the closest match:\n\n{}", shown.join("\n"))
}

// MARK: list_dir

pub struct ListDir(pub Arc<Ctx>);

impl Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }
    fn description(&self) -> &'static str {
        "List a directory's entries. Directories are marked with a trailing slash."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path (default current directory)." },
                "show_hidden": { "type": "boolean", "description": "Include dotfiles (default false)." }
            },
            "required": []
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let raw = str_arg(args, "path").filter(|s| !s.is_empty()).unwrap_or(".");
        let path = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&path) {
            return ToolResult::err(format!("Refusing to list {raw}: {reason}."));
        }
        if !path.is_dir() {
            return ToolResult::err(format!("Not a directory: {raw}"));
        }
        let Ok(entries) = std::fs::read_dir(&path) else {
            return ToolResult::err(format!("Cannot list: {raw}"));
        };
        let show_hidden = bool_arg(args, "show_hidden").unwrap_or(false);
        let mut rows: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !show_hidden && name.starts_with('.') {
                    return None;
                }
                Some(if entry.path().is_dir() { format!("{name}/") } else { name })
            })
            .collect();
        rows.sort();
        ToolResult::ok(if rows.is_empty() { "(empty)".to_string() } else { rows.join("\n") })
    }
}

// MARK: search_files

pub struct SearchFiles(pub Arc<Ctx>);

const FILE_SCAN_CAP: usize = 20_000;
/// Larger files are not searched for content: they are data, not source.
const CONTENT_SIZE_CAP: u64 = 2_000_000;
/// Folders that hold dependencies and build output, not the project's own code.
pub(crate) const SKIPPED_DIRS: [&str; 5] = [".git", "node_modules", "target", ".build", "__pycache__"];

impl Tool for SearchFiles {
    fn name(&self) -> &'static str {
        "search_files"
    }
    fn description(&self) -> &'static str {
        "Find files under a directory. Filter by `name` (substring of the filename) and/or `contains` \
         (substring within file text). Returns absolute paths that read_file opens as they are. \
         Dependency and build folders (.git, node_modules, target, .build) are skipped."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "directory": { "type": "string", "description": "Root directory to search (a leading ~ is expanded)." },
                "name": { "type": "string", "description": "Case-insensitive substring of the filename." },
                "contains": { "type": "string", "description": "Case-insensitive substring to find inside text files." },
                "max_results": { "type": "integer", "description": "Maximum matches to return (default 100)." }
            },
            "required": ["directory"]
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(raw) = str_arg(args, "directory").filter(|s| !s.is_empty()) else {
            return ToolResult::err("search_files requires a `directory`.");
        };
        let root = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&root) {
            return ToolResult::err(format!("Refusing to search {raw}: {reason}."));
        }
        let name = str_arg(args, "name").map(str::to_lowercase).filter(|s| !s.is_empty());
        let contains = str_arg(args, "contains").map(str::to_lowercase).filter(|s| !s.is_empty());
        if name.is_none() && contains.is_none() {
            return ToolResult::err("Provide `name` and/or `contains` to search for.");
        }
        if !root.is_dir() {
            return ToolResult::err(format!("Cannot search: {raw}"));
        }
        let max_results = int_arg(args, "max_results").unwrap_or(100).max(1) as usize;

        let mut matches: Vec<String> = Vec::new();
        let mut scanned = 0usize;
        let mut pending = vec![root];
        'walk: while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            let mut subdirs = Vec::new();
            for entry in entries {
                let Ok(kind) = entry.file_type() else { continue };
                let file_name = entry.file_name().to_string_lossy().into_owned();
                if kind.is_dir() {
                    if !SKIPPED_DIRS.contains(&file_name.as_str()) {
                        subdirs.push(entry.path());
                    }
                    continue;
                }
                if !kind.is_file() {
                    continue; // links are not followed
                }
                scanned += 1;
                if scanned > FILE_SCAN_CAP {
                    break 'walk;
                }
                if name.as_ref().is_some_and(|n| !file_name.to_lowercase().contains(n)) {
                    continue;
                }
                if let Some(needle) = &contains {
                    let small = entry.metadata().map(|m| m.len() <= CONTENT_SIZE_CAP).unwrap_or(false);
                    let found = small
                        && std::fs::read(entry.path())
                            .ok()
                            .and_then(|b| String::from_utf8(b).ok())
                            .is_some_and(|text| text.to_lowercase().contains(needle));
                    if !found {
                        continue;
                    }
                }
                matches.push(entry.path().to_string_lossy().into_owned());
                if matches.len() >= max_results {
                    break 'walk;
                }
            }
            // Depth-first in name order.
            pending.extend(subdirs.into_iter().rev());
        }

        if matches.is_empty() {
            return ToolResult::ok(format!("No matches under {raw}."));
        }
        let capped = if scanned > FILE_SCAN_CAP { format!("\n… [scan capped at {FILE_SCAN_CAP} files]") } else { String::new() };
        ToolResult::ok(matches.join("\n") + &capped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_of_lines_is_read_with_where_it_is() {
        let text = "a\nb\nc\nd\ne";
        assert_eq!(lines_of(text, 2, Some(3)), "[lines 2–3 of 5 — call again with start_line 4 for more]\nb\nc");
        assert_eq!(lines_of(text, 4, None), "[lines 4–5 of 5]\nd\ne");
        assert_eq!(lines_of(text, 3, Some(1)), "[lines 3–3 of 5 — call again with start_line 4 for more]\nc");
        assert!(lines_of(text, 9, None).contains("past its end"));
    }
    use crate::tools::testing::temp_dir;

    fn ctx(label: &str) -> Arc<Ctx> {
        Ctx::new(&temp_dir(label), false)
    }

    #[test]
    fn write_then_read_round_trips_and_creates_folders() {
        let c = ctx("write");
        let w = WriteFile(c.clone()).run(&json!({ "path": "a/b/note.txt", "content": "héllo\n" }));
        assert!(w.ok, "{}", w.text);
        assert_eq!(w.text, "Wrote 7 bytes to a/b/note.txt.");
        let r = ReadFile(c).run(&json!({ "path": "a/b/note.txt" }));
        assert_eq!(r.text, "héllo\n");
    }

    #[test]
    fn append_adds_to_the_end() {
        let c = ctx("append");
        WriteFile(c.clone()).run(&json!({ "path": "log.txt", "content": "one\n" }));
        let r = WriteFile(c.clone()).run(&json!({ "path": "log.txt", "content": "two\n", "append": "true" }));
        assert!(r.text.starts_with("Appended"));
        assert_eq!(std::fs::read_to_string(c.cwd.join("log.txt")).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn overwriting_a_file_that_changed_since_it_was_read_is_refused() {
        let c = ctx("stale");
        let file = c.cwd.join("f.txt");
        std::fs::write(&file, "v1").unwrap();
        ReadFile(c.clone()).run(&json!({ "path": "f.txt" }));
        // Someone else changes it: different content and a different time.
        std::fs::write(&file, "v2 from elsewhere").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        let r = WriteFile(c.clone()).run(&json!({ "path": "f.txt", "content": "mine" }));
        assert!(!r.ok && r.text.contains("changed on disk"), "{}", r.text);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2 from elsewhere");
        // After reading again the write goes through.
        ReadFile(c.clone()).run(&json!({ "path": "f.txt" }));
        assert!(WriteFile(c).run(&json!({ "path": "f.txt", "content": "mine" })).ok);
    }

    #[test]
    fn files_outside_the_project_are_refused() {
        let c = ctx("outside");
        let r = WriteFile(c.clone()).run(&json!({ "path": "../escape.txt", "content": "x" }));
        assert!(!r.ok && r.text.starts_with("Refusing to write"));
        assert!(!ReadFile(c).run(&json!({ "path": "/etc/hosts" })).ok);
    }

    #[test]
    fn edit_replaces_one_exact_match() {
        let c = ctx("edit");
        std::fs::write(c.cwd.join("m.py"), "def add(a, b):\n    return a - b\n").unwrap();
        let r = EditFile(c.clone()).run(&json!({ "path": "m.py", "old_text": "a - b", "new_text": "a + b" }));
        assert_eq!(r.text, "Replaced 1 occurrence in m.py.");
        assert_eq!(std::fs::read_to_string(c.cwd.join("m.py")).unwrap(), "def add(a, b):\n    return a + b\n");
    }

    #[test]
    fn edit_refuses_an_ambiguous_match_unless_replace_all() {
        let c = ctx("ambiguous");
        std::fs::write(c.cwd.join("t.txt"), "x = 1\nx = 1\n").unwrap();
        let e = EditFile(c.clone());
        let r = e.run(&json!({ "path": "t.txt", "old_text": "x = 1", "new_text": "x = 2" }));
        assert!(!r.ok && r.text.contains("occurs 2 times"));
        assert_eq!(std::fs::read_to_string(c.cwd.join("t.txt")).unwrap(), "x = 1\nx = 1\n");
        let r = e.run(&json!({ "path": "t.txt", "old_text": "x = 1", "new_text": "x = 2", "replace_all": true }));
        assert_eq!(r.text, "Replaced 2 occurrences in t.txt.");
    }

    #[test]
    fn edit_tolerates_trailing_whitespace_differences() {
        let c = ctx("tolerant");
        std::fs::write(c.cwd.join("t.txt"), "alpha  \nbeta\t\ngamma\n").unwrap();
        let r = EditFile(c.clone()).run(&json!({ "path": "t.txt", "old_text": "alpha\nbeta", "new_text": "ALPHA\nBETA" }));
        assert!(r.ok, "{}", r.text);
        assert_eq!(std::fs::read_to_string(c.cwd.join("t.txt")).unwrap(), "ALPHA\nBETA\ngamma\n");
    }

    #[test]
    fn a_missed_edit_shows_the_lines_near_the_closest_match() {
        let c = ctx("nearby");
        std::fs::write(c.cwd.join("t.txt"), "one\ntwo\nlet total = price * count\nfour\n").unwrap();
        let r = EditFile(c).run(&json!({ "path": "t.txt", "old_text": "let total = price + count", "new_text": "x" }));
        assert!(!r.ok);
        assert!(r.text.contains("3 | let total = price * count"), "{}", r.text);
    }

    #[test]
    fn list_marks_folders_and_hides_dotfiles() {
        let c = ctx("list");
        std::fs::create_dir(c.cwd.join("src")).unwrap();
        std::fs::write(c.cwd.join("b.txt"), "").unwrap();
        std::fs::write(c.cwd.join(".env"), "").unwrap();
        assert_eq!(ListDir(c.clone()).run(&json!({})).text, "b.txt\nsrc/");
        assert_eq!(ListDir(c).run(&json!({ "show_hidden": true })).text, ".env\nb.txt\nsrc/");
    }

    #[test]
    fn search_finds_by_name_and_content_and_skips_dependency_folders() {
        let c = ctx("search");
        std::fs::create_dir_all(c.cwd.join("src/deep")).unwrap();
        std::fs::create_dir_all(c.cwd.join("node_modules/lib")).unwrap();
        std::fs::write(c.cwd.join("src/deep/net.rs"), "use URLSession here").unwrap();
        std::fs::write(c.cwd.join("src/other.rs"), "nothing").unwrap();
        std::fs::write(c.cwd.join("node_modules/lib/net.rs"), "urlsession").unwrap();
        let s = SearchFiles(c.clone());
        let r = s.run(&json!({ "directory": ".", "name": ".RS", "contains": "urlsession" }));
        assert_eq!(r.text, c.cwd.join("src/deep/net.rs").to_string_lossy());
        assert_eq!(s.run(&json!({ "directory": ".", "name": "zzz" })).text, "No matches under ..");
        assert!(!s.run(&json!({ "directory": "." })).ok);
    }

    #[test]
    fn a_write_or_edit_that_breaks_the_syntax_leaves_the_file_as_it_was() {
        let c = ctx("verify");
        let good = "{\n  \"name\": \"shop\"\n}\n";
        std::fs::write(c.cwd.join("package.json"), good).unwrap();
        ReadFile(c.clone()).run(&json!({ "path": "package.json" }));

        let broken = WriteFile(c.clone()).run(&json!({ "path": "package.json", "content": "{ \"name\": " }));
        assert!(!broken.ok && broken.text.contains("REJECTED") && broken.text.contains("NOT changed"), "{}", broken.text);
        assert_eq!(std::fs::read_to_string(c.cwd.join("package.json")).unwrap(), good);

        let edit = EditFile(c.clone()).run(&json!({ "path": "package.json", "old_text": "\"shop\"", "new_text": "\"shop\"," }));
        assert!(!edit.ok && edit.text.contains("not valid JSON"), "{}", edit.text);
        assert_eq!(std::fs::read_to_string(c.cwd.join("package.json")).unwrap(), good);

        // Appending is judged on the whole file it would make.
        std::fs::write(c.cwd.join("list.json"), "[1, 2").unwrap();
        let appended = WriteFile(c.clone()).run(&json!({ "path": "list.json", "content": ", 3]", "append": true }));
        assert!(appended.ok, "{}", appended.text);
        // A new file that would be broken is not created.
        let created = WriteFile(c.clone()).run(&json!({ "path": "new.json", "content": "{" }));
        assert!(!created.ok && !c.cwd.join("new.json").exists());
        let _ = std::fs::remove_dir_all(&c.cwd);
    }
}
