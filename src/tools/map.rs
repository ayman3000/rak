//! code_map: the project at a glance, every source file with the functions,
//! types and classes it defines and their lines. On a large codebase the
//! model finds where things are in one call instead of a dozen searches and
//! reads, and reads only the lines it needs.
//!
//! Definitions are found line by line, by the words that start them in each
//! language (`fn`, `def`, `class`, `func`…), at the top level and one level
//! in. No parser: it can miss a definition written oddly, never invent one.

use super::fs::SKIPPED_DIRS;
use super::{str_arg, Ctx, Tool, ToolResult};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

/// The map stops growing here; a folder narrows it.
const MAX_CHARS: usize = 14_000;
const MAX_FILES: usize = 5_000;
const MAX_FILE_BYTES: u64 = 1_000_000;
/// Per file, after which "…" says there are more.
const MAX_DEFS: usize = 40;
/// Build output and environments, besides search_files' own list.
const MORE_SKIPPED: [&str; 8] = ["build", "dist", "DerivedData", ".venv", "venv", "Pods", ".next", ".dart_tool"];

/// Words that start a definition, per file extension.
fn keywords(ext: &str) -> &'static [&'static str] {
    match ext {
        "rs" => &["fn", "struct", "enum", "trait", "impl", "mod", "type", "macro_rules!"],
        "py" => &["def", "class"],
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => &["function", "class", "interface", "type", "enum"],
        "swift" => &["func", "class", "struct", "enum", "protocol", "extension", "actor"],
        "go" => &["func", "type"],
        "kt" | "kts" => &["fun", "class", "interface", "object", "enum class", "data class"],
        "java" | "cs" => &["class", "interface", "enum", "record"],
        "dart" => &["class", "enum", "mixin", "extension"],
        "rb" => &["def", "class", "module"],
        "php" => &["function", "class", "interface", "trait"],
        _ => &[],
    }
}

/// Words that may come before the defining word.
const MODIFIERS: [&str; 20] = [
    "pub(crate)",
    "pub(super)",
    "pub",
    "export",
    "default",
    "async",
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "final",
    "open",
    "override",
    "fileprivate",
    "abstract",
    "unsafe",
    "sealed",
    "declare",
    "@MainActor",
];

/// Pure — unit-tested. The definitions in a file's text: (what, line).
pub fn outline(ext: &str, text: &str) -> Vec<(String, usize)> {
    let words = keywords(ext);
    if words.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let indent = line.len() - line.trim_start().len();
        if indent > 4 || line.contains('\t') && indent > 1 {
            continue;
        }
        let mut rest = line.trim();
        // const NAME = (…) => / function expressions count as functions in JS.
        if matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs") && indent == 0 {
            let bare = rest.trim_start_matches("export ");
            if let Some(decl) = bare.strip_prefix("const ").or_else(|| bare.strip_prefix("let ")) {
                if let Some((name, value)) = decl.split_once('=') {
                    let value = value.trim_start().trim_start_matches("async ");
                    if value.starts_with('(') && value.contains("=>") || value.starts_with("function") {
                        out.push((format!("function {}", name.trim().trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_')), n + 1));
                        continue;
                    }
                }
            }
        }
        while let Some(stripped) = MODIFIERS.iter().find_map(|m| rest.strip_prefix(m).and_then(|r| r.strip_prefix(' '))) {
            rest = stripped.trim_start();
        }
        let Some((word, after)) = words.iter().find_map(|w| rest.strip_prefix(w).filter(|a| a.starts_with(' ')).map(|a| (*w, a.trim())))
        else {
            continue;
        };
        // Go methods: `func (s *Server) Start()` → Start.
        let after = if word == "func" && after.starts_with('(') { after.split_once(')').map_or("", |(_, r)| r.trim()) } else { after };
        let name: String = if word == "impl" || word == "extension" {
            // `impl Tool for Todo {` → "Tool for Todo"
            after.split(['{', ':']).next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            after.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '!').collect()
        };
        if !name.is_empty() && name != "<" {
            out.push((format!("{word} {name}"), n + 1));
        }
    }
    out
}

pub struct CodeMap(pub Arc<Ctx>);

impl Tool for CodeMap {
    fn name(&self) -> &str {
        "code_map"
    }
    fn description(&self) -> &str {
        "Map a codebase: every source file with the functions, types and classes it defines, and their line numbers. \
         Use it first in an unfamiliar or large project, then read only the lines you need."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "Folder to map (default: the project)." } },
        })
    }
    fn run(&self, args: &Value) -> ToolResult {
        let raw = str_arg(args, "path").filter(|p| !p.trim().is_empty()).unwrap_or(".");
        let root = self.0.resolve(raw);
        if let Some(reason) = self.0.block_reason(&root) {
            return ToolResult::err(format!("Refusing to map {raw}: {reason}."));
        }
        if !root.is_dir() {
            return ToolResult::err(format!("{raw} is not a folder."));
        }
        ToolResult::ok(map(&root))
    }
}

/// Pure but for reading the folder — unit-tested.
pub fn map(root: &Path) -> String {
    let mut out = String::new();
    let (mut files, mut cut_short) = (0usize, false);
    let mut other: Vec<String> = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    'walk: while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let mut subdirs = Vec::new();
        for entry in entries {
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_str()) && !MORE_SKIPPED.contains(&name.as_str()) && !name.starts_with('.') {
                    subdirs.push(entry.path());
                }
                continue;
            }
            if !kind.is_file() || name.starts_with('.') {
                continue;
            }
            files += 1;
            if files > MAX_FILES || out.len() > MAX_CHARS {
                cut_short = true;
                break 'walk;
            }
            let path = entry.path();
            let shown = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let small = entry.metadata().map(|m| m.len() <= MAX_FILE_BYTES).unwrap_or(false);
            let defs = if small && !keywords(&ext).is_empty() {
                std::fs::read_to_string(&path).map(|text| outline(&ext, &text)).unwrap_or_default()
            } else {
                Vec::new()
            };
            if defs.is_empty() {
                other.push(shown);
                continue;
            }
            let mut listed: Vec<String> = defs.iter().take(MAX_DEFS).map(|(what, line)| format!("{what} {line}")).collect();
            if defs.len() > MAX_DEFS {
                listed.push(format!("… {} more", defs.len() - MAX_DEFS));
            }
            out.push_str(&format!("{shown}: {}\n", listed.join(", ")));
        }
        pending.extend(subdirs.into_iter().rev());
    }
    if !other.is_empty() {
        let mut list = other.join(", ");
        if list.len() > 3_000 {
            list = format!("{}… ({} files)", list.chars().take(3_000).collect::<String>(), other.len());
        }
        out.push_str(&format!("Other files: {list}\n"));
    }
    if out.is_empty() {
        return "No files here (dependency, build and hidden folders are skipped).".to_string();
    }
    if cut_short {
        out.push_str("[The map stops here: the project is big. Map a folder with `path` to see the rest.]\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    fn names(ext: &str, text: &str) -> Vec<String> {
        outline(ext, text).into_iter().map(|(what, line)| format!("{what}@{line}")).collect()
    }

    #[test]
    fn definitions_are_found_in_each_language() {
        let rust = "pub struct Agent {\n    x: u8,\n}\nimpl Tool for Todo {\n    fn name(&self) {}\n        fn too_deep() {}\n}\npub(crate) async fn run() {}\n// fn not_code\n";
        assert_eq!(names("rs", rust), ["struct Agent@1", "impl Tool for Todo@4", "fn name@5", "fn run@8"]);
        assert_eq!(
            names("py", "class Shop:\n    def buy(self):\n        pass\nasync def main():\n"),
            ["class Shop@1", "def buy@2", "def main@4"]
        );
        let ts = "export default class App {}\nexport const load = async (id: string) => {}\nconst x = 5\ninterface Props {}\n";
        assert_eq!(names("ts", ts), ["class App@1", "function load@2", "interface Props@4"]);
        assert_eq!(names("swift", "@MainActor final class Model {\n    private func fetch() {}\n}\n"), ["class Model@1", "func fetch@2"]);
        assert_eq!(
            names("go", "func (s *Server) Start() {}\ntype Config struct {}\nfunc main() {}\n"),
            ["func Start@1", "type Config@2", "func main@3"]
        );
        assert!(outline("txt", "fn looks_like_code()").is_empty());
    }

    #[test]
    fn a_project_is_mapped_with_dependencies_and_hidden_folders_left_out() {
        let dir = temp_dir("code-map");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/lib")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/app.py"), "class App:\n    def run(self):\n        pass\n").unwrap();
        std::fs::write(dir.join("node_modules/lib/x.js"), "function hidden() {}").unwrap();
        std::fs::write(dir.join("README.md"), "# App").unwrap();
        let text = map(&dir);
        assert_eq!(text, "src/app.py: class App 1, def run 2\nOther files: README.md\n");
        let tool = CodeMap(Ctx::new(&dir, false));
        assert!(tool.run(&json!({})).text.contains("def run 2"));
        assert!(!tool.run(&json!({ "path": "/etc" })).ok);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
