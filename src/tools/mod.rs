//! The tools the model can call, and what they share: where the project is,
//! which paths are allowed, and which files this run has already seen.

pub mod fs;
pub mod image;
pub mod map;
pub mod shell;
pub mod todo;
pub mod verify;
pub mod web;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub struct ToolResult {
    pub ok: bool,
    pub text: String,
    /// An image for the model to look at (view_image).
    pub image: Option<Image>,
}

/// An image, base64-encoded, with its media type ("image/png").
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub media_type: String,
    pub data: String,
}

impl Image {
    /// The image as a data URL, the form chat-completions servers take.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.data)
    }
}

impl ToolResult {
    pub fn ok(text: impl Into<String>) -> Self {
        ToolResult { ok: true, text: text.into(), image: None }
    }
    pub fn err(text: impl Into<String>) -> Self {
        ToolResult { ok: false, text: text.into(), image: None }
    }
}

pub trait Tool {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema for the arguments.
    fn parameters(&self) -> Value;
    /// True for tools that change files or run commands.
    fn needs_approval(&self) -> bool {
        false
    }
    fn run(&self, args: &Value) -> ToolResult;
}

/// The tool in the shape chat-completions servers expect.
pub fn definition(tool: &dyn Tool) -> Value {
    json!({
        "type": "function",
        "function": { "name": tool.name(), "description": tool.description(), "parameters": tool.parameters() }
    })
}

/// The standard set. Read-only leaves out everything that changes the machine.
pub fn core_tools(ctx: &Arc<Ctx>, read_only: bool) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(fs::ListDir(ctx.clone())),
        Box::new(fs::SearchFiles(ctx.clone())),
        Box::new(map::CodeMap(ctx.clone())),
        Box::new(fs::ReadFile(ctx.clone())),
    ];
    if !read_only {
        tools.push(Box::new(fs::WriteFile(ctx.clone())));
        tools.push(Box::new(fs::EditFile(ctx.clone())));
        tools.push(Box::new(shell::RunShell::new(ctx.clone())));
    }
    tools
}

/// Shared by every tool in one run.
pub struct Ctx {
    /// The project folder, symlinks resolved. Relative paths resolve here.
    pub cwd: PathBuf,
    /// False: file tools only reach inside `cwd`.
    pub any_path: bool,
    /// Files read or written in this run, with their modification time then.
    seen: Mutex<HashMap<PathBuf, Option<SystemTime>>>,
    /// Set by Ctrl-C: a running command is killed and the request ends.
    pub stop: crate::interrupt::Stop,
}

impl Ctx {
    pub fn new(cwd: &Path, any_path: bool) -> Arc<Self> {
        Arc::new(Ctx { cwd: real_path(cwd), any_path, seen: Mutex::new(HashMap::new()), stop: Default::default() })
    }

    /// `~` expanded, relative paths joined to the project folder, `.` and `..`
    /// folded away.
    pub fn resolve(&self, raw: &str) -> PathBuf {
        let expanded = if raw == "~" {
            home()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            home().join(rest)
        } else {
            PathBuf::from(raw)
        };
        let joined = if expanded.is_absolute() { expanded } else { self.cwd.join(expanded) };
        normalize(&joined)
    }

    /// Why this path may not be touched, or None when it may. Symlinks are
    /// resolved first, so a link inside the project that points outside it is
    /// treated as outside.
    pub fn block_reason(&self, path: &Path) -> Option<String> {
        if self.any_path || real_path(path).starts_with(&self.cwd) {
            return None;
        }
        Some(format!("it is outside the project folder {} (start rak with --any-path to allow that)", self.cwd.display()))
    }

    pub fn note_seen(&self, path: &Path) {
        self.seen.lock().unwrap().insert(path.to_path_buf(), modified(path));
    }

    /// Set when the file changed on disk after this run last read or wrote it:
    /// overwriting it now would throw that change away.
    pub fn stale_reason(&self, path: &Path) -> Option<String> {
        let seen = self.seen.lock().unwrap();
        let then = seen.get(path)?;
        (*then != modified(path)).then(|| "it changed on disk since you last read it".to_string())
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Folds `.` and `..` without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The path with symlinks resolved. The path may not exist yet (a file about
/// to be written): the deepest folder that does exist is resolved and the rest
/// is kept as written.
pub fn real_path(path: &Path) -> PathBuf {
    let path = normalize(path);
    let mut existing = path.as_path();
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            return tail.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name);
                existing = parent;
            }
            _ => return path,
        }
    }
}

// Argument readers. Models send numbers as strings and booleans as words often
// enough that the tools accept both.

pub fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub fn int_arg(args: &Value, key: &str) -> Option<i64> {
    match args.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn bool_arg(args: &Value, key: &str) -> Option<bool> {
    match args.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    /// A fresh empty folder for one test.
    pub fn temp_dir(label: &str) -> PathBuf {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("rak-test-{}-{label}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        super::real_path(&dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_in_the_project_and_dots_fold() {
        let dir = testing::temp_dir("resolve");
        let ctx = Ctx::new(&dir, false);
        assert_eq!(ctx.resolve("a/./b/../c.txt"), dir.join("a/c.txt"));
        assert_eq!(ctx.resolve(dir.join("x").to_str().unwrap()), dir.join("x"));
    }

    #[test]
    fn paths_outside_the_project_are_blocked_unless_allowed() {
        let dir = testing::temp_dir("policy");
        let ctx = Ctx::new(&dir, false);
        assert!(ctx.block_reason(&ctx.resolve("inside/new.txt")).is_none());
        assert!(ctx.block_reason(&ctx.resolve("../elsewhere.txt")).is_some());
        assert!(Ctx::new(&dir, true).block_reason(&ctx.resolve("../elsewhere.txt")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_leaves_the_project_counts_as_outside() {
        let dir = testing::temp_dir("link");
        let outside = testing::temp_dir("link-target");
        std::os::unix::fs::symlink(&outside, dir.join("escape")).unwrap();
        let ctx = Ctx::new(&dir, false);
        assert!(ctx.block_reason(&ctx.resolve("escape/secret.txt")).is_some());
    }

    #[test]
    fn arguments_are_read_from_numbers_or_strings() {
        let args = json!({ "a": 5, "b": "12", "c": "true", "d": false });
        assert_eq!(int_arg(&args, "a"), Some(5));
        assert_eq!(int_arg(&args, "b"), Some(12));
        assert_eq!(bool_arg(&args, "c"), Some(true));
        assert_eq!(bool_arg(&args, "d"), Some(false));
        assert_eq!(int_arg(&args, "missing"), None);
    }
}
