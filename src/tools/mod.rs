//! The tools the model can call, and what they share: where the project is,
//! which paths are allowed, and which files this run has already seen.

pub mod chats;
pub mod fetch;
pub mod fs;
pub mod image;
pub mod map;
pub mod patch;
pub mod shell;
pub mod todo;
pub mod user;
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
    /// True when several calls of this tool from one reply should go to
    /// `run_together` (delegate's helpers run in parallel).
    fn runs_together(&self) -> bool {
        false
    }
    /// Several calls from one reply, one result each, in order.
    fn run_together(&self, calls: &[Value]) -> Vec<ToolResult> {
        calls.iter().map(|args| self.run(args)).collect()
    }
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
    let mut tools: Vec<Box<dyn Tool>> = reading_tools(ctx).into_iter().map(|t| t as Box<dyn Tool>).collect();
    if !read_only {
        tools.push(Box::new(fs::WriteFile(ctx.clone())));
        tools.push(Box::new(fs::EditFile(ctx.clone())));
        tools.push(Box::new(patch::ApplyPatch(ctx.clone())));
        tools.push(Box::new(shell::RunShell::new(ctx.clone())));
    }
    tools
}

/// The tools that only read, in a form that can move to another thread
/// (delegate's helpers run on their own).
pub fn reading_tools(ctx: &Arc<Ctx>) -> Vec<Box<dyn Tool + Send>> {
    vec![
        Box::new(fs::ListDir(ctx.clone())),
        Box::new(fs::SearchFiles(ctx.clone())),
        Box::new(map::CodeMap(ctx.clone())),
        Box::new(fs::ReadFile(ctx.clone())),
        Box::new(fetch::FetchUrl::new()),
    ]
}

/// Shared by every tool in one run.
pub struct Ctx {
    /// The project folder, symlinks resolved. Relative paths resolve here.
    pub cwd: PathBuf,
    /// False: file tools only reach inside `cwd` and the folders in `extra`.
    pub any_path: bool,
    /// Folders the person added for this run (`/add-dir`, `--add-dir`),
    /// symlinks resolved. Only the person adds one; no tool does.
    extra: Mutex<Vec<PathBuf>>,
    /// Files read or written in this run, with their modification time then.
    seen: Mutex<HashMap<PathBuf, Option<SystemTime>>>,
    /// Set by Ctrl-C: a running command is killed and the request ends.
    pub stop: crate::interrupt::Stop,
    /// Commands run_shell started in the background (servers), for /jobs.
    pub jobs: shell::Jobs,
}

impl Ctx {
    pub fn new(cwd: &Path, any_path: bool) -> Arc<Self> {
        Arc::new(Ctx {
            cwd: real_path(cwd),
            any_path,
            extra: Mutex::new(Vec::new()),
            seen: Mutex::new(HashMap::new()),
            stop: Default::default(),
            jobs: Default::default(),
        })
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
        let real = real_path(path);
        // The room of agents in talk mode: only rak writes there.
        if real.starts_with(self.cwd.join(".rak").join("talk")) {
            return Some(
                "it is in .rak/talk, the room where agents in this folder leave each other messages. Only rak reads and writes \
                 there: use send_message and wait_message when you have them, and leave the folder alone."
                    .to_string(),
            );
        }
        if self.any_path || real.starts_with(&self.cwd) || self.extra.lock().unwrap().iter().any(|folder| real.starts_with(folder)) {
            return None;
        }
        if let Some(why) = never_reason(&real, &secret_folders(&real_path(&home()))) {
            return Some(why);
        }
        // Said so the model stops here: going round it (a shell command, a
        // copy inside the project) is not what the person asked for.
        Some(format!(
            "it is outside the project folder {}. Only the user can allow another folder: stop, and tell them in one or two \
             lines to type /add-dir PATH in the chat (or start rak with --add-dir PATH). Do not work around this.",
            self.cwd.display()
        ))
    }

    /// The folders added to this run, as they were resolved.
    pub fn extra_folders(&self) -> Vec<PathBuf> {
        self.extra.lock().unwrap().clone()
    }

    /// Lets the file tools reach this folder too, for this run. `protected`:
    /// places never opened this way (where rak keeps its keys). The folder as
    /// resolved comes back, to show the person what they opened.
    pub fn add_folder(&self, raw: &str, protected: &[PathBuf]) -> Result<PathBuf, String> {
        let typed = self.resolve(raw.trim());
        let real = std::fs::canonicalize(&typed).map_err(|_| format!("{} is not a folder that exists", typed.display()))?;
        if !real.is_dir() {
            return Err(format!("{} is a file, not a folder", real.display()));
        }
        if real.starts_with(&self.cwd) {
            return Err(format!("{} is inside the project already", real.display()));
        }
        let mut never: Vec<PathBuf> = protected.iter().map(|p| real_path(p)).collect();
        never.extend(secret_folders(&real_path(&home())));
        if let Some(why) = refuse_folder(&real, &never) {
            return Err(why);
        }
        let mut extra = self.extra.lock().unwrap();
        if !extra.contains(&real) {
            extra.push(real.clone());
        }
        Ok(real)
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

/// Where keys and credentials usually live under a home folder.
fn secret_folders(home: &Path) -> Vec<PathBuf> {
    [".ssh", ".gnupg", ".aws", ".kube", ".docker", ".config/rak"].iter().map(|name| home.join(name)).collect()
}

/// Pure — unit-tested. What the model is told about a path inside a place
/// where keys live, or None for any other path. /add-dir refuses these
/// folders, so the model must not send the person to it; for rak's own
/// folder it is pointed at the tool that reads earlier chats.
pub fn never_reason(path: &Path, never: &[PathBuf]) -> Option<String> {
    let secret = never.iter().find(|secret| path.starts_with(secret))?;
    let chats =
        if secret.ends_with(".config/rak") { " For this project's earlier chats, use the past_chats tool when you have it." } else { "" };
    Some(format!(
        "it is in {}, where keys are kept. This folder is never opened, and /add-dir does not open it either: do not ask the user to add it, and do not work around this. Say in one line that you can't read it.{chats}",
        secret.display()
    ))
}

/// Pure — unit-tested. Why a folder is never added, or None when it may be:
/// the disk's root, and any folder that holds, is, or is inside a place
/// where keys live. So the home folder itself is refused (it holds ~/.ssh),
/// and a project under it is not.
pub fn refuse_folder(folder: &Path, never: &[PathBuf]) -> Option<String> {
    if folder.parent().is_none() {
        return Some("the whole disk is never added; name the folder you mean".to_string());
    }
    let secret = never.iter().find(|secret| folder.starts_with(secret) || secret.starts_with(folder))?;
    Some(if folder.starts_with(secret) {
        format!("{} is where keys are kept; it is never added", secret.display())
    } else {
        format!("{} holds {}, where keys are kept; name a folder inside it instead", folder.display(), secret.display())
    })
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

    #[test]
    fn a_folder_the_person_adds_is_reachable_and_nothing_else_is() {
        let dir = testing::temp_dir("add-project");
        let other = testing::temp_dir("add-other");
        let third = testing::temp_dir("add-third");
        std::fs::write(other.join("a.txt"), "x").unwrap();
        let ctx = Ctx::new(&dir, false);
        let inside = other.join("src/new.rs");
        let why = ctx.block_reason(&inside).unwrap();
        assert!(why.contains("outside the project") && why.contains("/add-dir") && why.contains("stop"), "{why}");
        assert_eq!(ctx.add_folder(other.to_str().unwrap(), &[]).unwrap(), other);
        assert!(ctx.block_reason(&inside).is_none(), "a new file in the added folder");
        assert!(ctx.block_reason(&third.join("x")).is_some(), "only that folder was added");
        assert!(ctx.block_reason(&other.parent().unwrap().join("sibling.txt")).is_some(), "not its parent");
        assert_eq!(ctx.extra_folders(), std::slice::from_ref(&other));
        ctx.add_folder(other.to_str().unwrap(), &[]).unwrap();
        assert_eq!(ctx.extra_folders().len(), 1, "adding it twice keeps one");
        // What is refused: a file, a folder that is not there, one inside the project, a protected one.
        assert!(ctx.add_folder(other.join("a.txt").to_str().unwrap(), &[]).unwrap_err().contains("a file"));
        assert!(ctx.add_folder(other.join("nope").to_str().unwrap(), &[]).unwrap_err().contains("not a folder that exists"));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert!(ctx.add_folder("sub", &[]).unwrap_err().contains("inside the project"));
        assert!(ctx.add_folder(third.to_str().unwrap(), &[third.join("keys")]).unwrap_err().contains("holds"));
        assert_eq!(ctx.extra_folders().len(), 1);
    }

    #[test]
    fn the_disk_the_home_folder_and_where_keys_live_are_never_added() {
        let home = Path::new("/Users/me");
        let never = secret_folders(home);
        assert!(refuse_folder(Path::new("/"), &never).unwrap().contains("whole disk"));
        assert!(refuse_folder(home, &never).unwrap().contains("holds /Users/me/.ssh"));
        assert!(refuse_folder(Path::new("/Users"), &never).is_some(), "a folder above the home folder holds it too");
        assert!(refuse_folder(&home.join(".ssh"), &never).unwrap().contains("never added"));
        assert!(refuse_folder(&home.join(".ssh/keys"), &never).is_some());
        assert!(refuse_folder(&home.join(".config"), &never).unwrap().contains(".config/rak"));
        assert!(refuse_folder(&home.join(".config/rak/sessions"), &never).is_some());
        assert_eq!(refuse_folder(&home.join("projects/shell-coach"), &never), None);
        assert_eq!(refuse_folder(&home.join("projects"), &never), None);
        assert_eq!(refuse_folder(Path::new("/tmp/work"), &never), None);
    }

    #[test]
    fn the_room_of_agents_is_closed_to_the_file_tools() {
        let dir = testing::temp_dir("room-closed");
        for any_path in [false, true] {
            let ctx = Ctx::new(&dir, any_path);
            for inside in [".rak/talk", ".rak/talk/talk.md", ".rak/talk/msg/000001.json", ".rak/talk/here/A.json"] {
                let why = ctx.block_reason(&ctx.resolve(inside)).unwrap_or_else(|| panic!("{inside} was open (any_path {any_path})"));
                assert!(why.contains("send_message") && why.contains("wait_message"), "{why}");
            }
            // The rest of .rak, and a file that only looks alike, stay as they were.
            assert_eq!(ctx.block_reason(&ctx.resolve(".rak/model")), None);
            assert_eq!(ctx.block_reason(&ctx.resolve(".rak/talking.md")), None);
            assert_eq!(ctx.block_reason(&ctx.resolve("talk/notes.md")), None);
        }
    }

    #[test]
    fn a_path_where_keys_live_is_never_sent_to_add_dir() {
        let home = Path::new("/Users/me");
        let never = secret_folders(home);
        for name in [".ssh/id_ed25519", ".gnupg/x", ".aws/credentials", ".kube/config", ".docker/config.json"] {
            let why = never_reason(&home.join(name), &never).unwrap();
            assert!(why.contains("never opened") && why.contains("do not ask"), "{why}");
            assert!(!why.contains("past_chats"), "{why}");
        }
        let why = never_reason(&home.join(".config/rak/sessions/rak-1/chat.json"), &never).unwrap();
        assert!(why.contains("/Users/me/.config/rak,") && why.contains("past_chats"), "{why}");
        assert_eq!(never_reason(&home.join("projects/other/file.txt"), &never), None);
        assert_eq!(never_reason(&home.join(".config/other"), &never), None);
        assert_eq!(never_reason(home, &never), None, "the home folder itself gets the usual message");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_an_added_folder_is_shown_as_where_it_leads() {
        let dir = testing::temp_dir("add-link");
        let target = testing::temp_dir("add-link-target");
        std::os::unix::fs::symlink(&target, dir.parent().unwrap().join(format!("{}-link", dir.file_name().unwrap().to_string_lossy())))
            .unwrap();
        let ctx = Ctx::new(&dir, false);
        let link = format!("../{}-link", dir.file_name().unwrap().to_string_lossy());
        assert_eq!(ctx.add_folder(&link, &[]).unwrap(), target, "the person sees the real folder, not the link");
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

#[cfg(test)]
mod sizes {
    #[test]
    #[ignore]
    fn print_definition_sizes() {
        let ctx = super::Ctx::new(&super::testing::temp_dir("sizes"), false);
        let mut tools = super::core_tools(&ctx, false);
        tools.push(Box::new(crate::memory::Remember { project_file: "a".into(), user_file: None }));
        tools.push(Box::new(crate::routines::SaveRoutine { project: "a".into(), user: None }));
        for t in &tools {
            println!("{:>6} {}", super::definition(t.as_ref()).to_string().len(), t.name());
        }
    }
}
