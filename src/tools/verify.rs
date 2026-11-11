//! The check a file passes before write_file or edit_file puts it on disk.
//! Ported from SwiftAgentKit's WriteVerifier.
//!
//! Content sometimes arrives damaged (a long argument degraded on the way,
//! unified-diff `+` markers leaking into the file), and a model sometimes
//! writes code that does not parse. Either way the file is left as it was and
//! the model is told why, in words that say whether resending can help.
//!
//! Rules, from SwiftAgentKit:
//! - A false rejection is worse than a miss: it blocks the agent. The only
//!   heuristic is one high-precision signature (many leaked diff markers);
//!   everything else is judged by the language's real parser, or passes.
//! - A missing parser, a timeout or a crash means the check is skipped,
//!   never a rejection.
//! - When a machine has several copies of an interpreter, the newest judges:
//!   an old system Python must not reject code written for the one in use.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Files above this size are not given to a parser.
pub const MAX_CHECKED_BYTES: usize = 2_000_000;
const PARSER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq)]
pub enum Rejection {
    /// The bytes were damaged on the way in. Resending can fix this.
    Corrupted(String),
    /// The bytes arrived intact and the language rejected them. Resending
    /// the same content fails the same way.
    Syntax { reason: String, judged_by: String },
}

impl Rejection {
    pub fn reason(&self) -> &str {
        match self {
            Rejection::Corrupted(reason) | Rejection::Syntax { reason, .. } => reason,
        }
    }

    /// What the model should do about it.
    pub fn guidance(&self) -> String {
        match self {
            Rejection::Corrupted(_) => "The file was NOT changed. The content arrived corrupted in transit — resend the write; \
                for large files write in chunks: an initial write_file under ~4KB, then append: true pieces, each under ~4KB."
                .to_string(),
            Rejection::Syntax { judged_by, .. } => format!(
                "The file was NOT changed. This is NOT transit corruption: the content arrived intact and {judged_by} rejected it, \
                 so resending the same content will fail in exactly the same way. Fix the syntax. If the code is correct for a \
                 newer version of the language, write for the version above — that is the one this machine will run the file with."
            ),
        }
    }

    /// The whole message for the model.
    pub fn message(&self, path: &str) -> String {
        format!("Write to {path} REJECTED — {}.\n\n{}", self.reason(), self.guidance())
    }
}

/// A parser: the program names to look for, the arguments before the file,
/// and the argument that makes it print its version (to pick the newest).
struct Parser {
    names: &'static [&'static str],
    args: &'static [&'static str],
    version: Option<&'static str>,
}

fn parser_for(extension: &str) -> Option<Parser> {
    // Python's check compiles in memory: py_compile would leave a __pycache__.
    const PY: &[&str] = &["-c", "import sys; compile(open(sys.argv[1], encoding='utf-8').read(), sys.argv[1], 'exec')"];
    Some(match extension {
        "py" => Parser { names: &["python3"], args: PY, version: Some("-V") },
        "js" | "mjs" | "cjs" => Parser { names: &["node"], args: &["--check"], version: Some("--version") },
        "sh" | "bash" => Parser { names: &["bash"], args: &["-n"], version: Some("--version") },
        "swift" => Parser { names: &["swiftc"], args: &["-parse"], version: None },
        "dart" => Parser { names: &["dart"], args: &["format", "--output=none"], version: Some("--version") },
        _ => return None,
    })
}

/// Extensions treated as code for the leaked-diff-marker check.
const CODE: &[&str] = &[
    "dart", "swift", "py", "js", "ts", "tsx", "jsx", "mjs", "cjs", "java", "kt", "c", "cc", "cpp", "h", "hpp", "m", "mm", "rs", "go", "rb",
    "json", "sh", "php", "cs", "scala", "lua",
];

/// None: the content may be written (or could not be judged). Otherwise why not.
pub fn check(path: &Path, content: &str) -> Option<Rejection> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    // 1. Diff markers leaking into a code file: many lines starting with "+".
    if CODE.contains(&extension.as_str()) {
        let lines: Vec<&str> = content.split('\n').collect();
        let leaked = lines.iter().filter(|l| l.starts_with('+') && !l.starts_with("++")).count();
        if leaked >= 5 && leaked as f64 / lines.len() as f64 > 0.2 {
            return Some(Rejection::Corrupted(format!(
                "content contains {leaked} lines with leaked unified-diff '+' prefixes — this is patch syntax bleeding into \
                 file content (transit corruption)"
            )));
        }
    }

    // 2. JSON is parsed here: fast, and always available. Files that by
    //    convention allow comments (tsconfig, VS Code settings, .jsonc) may
    //    have them, and trailing commas.
    if extension == "json" || extension == "jsonc" {
        let with_comments = allows_json_comments(path);
        let text = if with_comments { strip_json_comments(content) } else { content.to_string() };
        if serde_json::from_str::<serde_json::Value>(&text).is_err() {
            let reason = if with_comments {
                "content is not valid JSON (comments and trailing commas are fine in this file)"
            } else {
                "content is not valid JSON"
            };
            return Some(Rejection::Syntax { reason: reason.to_string(), judged_by: "the JSON parser".to_string() });
        }
        return None;
    }

    // 3. The language's own parser, when this machine has one.
    let parser = parser_for(&extension)?;
    if content.len() > MAX_CHECKED_BYTES {
        return None;
    }
    let program = resolve(parser.names, parser.version)?;
    let file = TempFile::new(&extension, content)?;
    let mut command = Command::new(&program);
    command.args(parser.args).arg(&file.0);
    match run_with_timeout(command, PARSER_TIMEOUT) {
        Outcome::Failed(diagnostics) => {
            let judge = match parser.version.and_then(|flag| version_banner(&program, flag)) {
                Some(banner) => format!("{program} ({banner})"),
                None => program.clone(),
            };
            // The temporary file's name means nothing to the model.
            let diagnostics = diagnostics.replace(&file.0.to_string_lossy().to_string(), &path.to_string_lossy());
            Some(Rejection::Syntax { reason: format!("content failed the syntax check run by {judge}:\n{diagnostics}"), judged_by: judge })
        }
        Outcome::Passed(_) | Outcome::Unavailable => None,
    }
}

/// Files whose format is "JSON with comments" by convention.
pub fn allows_json_comments(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    let in_vscode = path.components().any(|c| c.as_os_str() == ".vscode");
    name.ends_with(".jsonc")
        || ((name.starts_with("tsconfig") || name.starts_with("jsconfig")) && name.ends_with(".json"))
        || ["devcontainer.json", ".devcontainer.json", ".eslintrc.json", "deno.json", ".babelrc.json"].contains(&name.as_str())
        || (in_vscode && name.ends_with(".json"))
}

/// Pure — unit-tested. JSON with comments → JSON: `//` and `/* */` comments
/// outside strings are dropped, and so are commas right before `}` or `]`.
pub fn strip_json_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
        } else if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    let chars: Vec<char> = out.chars().collect();
    let mut result = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for (j, &c) in chars.iter().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
        } else if c == '"' {
            quoted = true;
        } else if c == ',' {
            let next = chars[j + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        result.push(c);
    }
    result
}

// MARK: - Finding the interpreter

/// Program name → its path on this machine (None: not there). Looked up once.
fn found() -> &'static Mutex<HashMap<String, Option<String>>> {
    static FOUND: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    FOUND.get_or_init(|| Mutex::new(HashMap::new()))
}

fn resolve(names: &[&str], version_flag: Option<&str>) -> Option<String> {
    for name in names {
        if let Some(cached) = found().lock().unwrap().get(*name) {
            if cached.is_some() {
                return cached.clone();
            }
            continue;
        }
        let best = newest(&candidates(name), version_flag);
        found().lock().unwrap().insert(name.to_string(), best.clone());
        if best.is_some() {
            return best;
        }
    }
    None
}

/// Every copy of a program: the usual install folders, then every folder on PATH.
fn candidates(name: &str) -> Vec<String> {
    let mut folders: Vec<std::path::PathBuf> = ["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"].iter().map(Into::into).collect();
    if let Some(path) = std::env::var_os("PATH") {
        folders.extend(std::env::split_paths(&path));
    }
    let mut out: Vec<String> = Vec::new();
    for folder in folders {
        let candidate = folder.join(name);
        if is_executable(&candidate) {
            let text = candidate.to_string_lossy().to_string();
            if !out.contains(&text) {
                out.push(text);
            }
        }
    }
    out
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The newest of several copies, by the version each reports. macOS ships
/// Python 3.9 as /usr/bin/python3; a file using `match` (3.10) must not be
/// judged by it when a newer Python is installed.
fn newest(candidates: &[String], version_flag: Option<&str>) -> Option<String> {
    let flag = match version_flag {
        Some(flag) if candidates.len() > 1 => flag,
        _ => return candidates.first().cloned(),
    };
    let ranked: Vec<(Vec<u64>, &String)> =
        candidates.iter().map(|c| (version_banner(c, flag).map(|b| parse_version(&b)).unwrap_or_default(), c)).collect();
    if ranked.iter().all(|(v, _)| v.is_empty()) {
        return candidates.first().cloned();
    }
    // max_by keeps the last of equals; walk in reverse so the first listed wins a tie.
    ranked.into_iter().rev().max_by(|a, b| compare_versions(&a.0, &b.0)).map(|(_, c)| c.clone())
}

/// Pure — unit-tested. The first dotted number in a banner: "Python 3.12.8" → [3, 12, 8].
pub fn parse_version(banner: &str) -> Vec<u64> {
    let chars: Vec<char> = banner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            let parts: Vec<u64> = run.split('.').filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect();
            if parts.len() > 1 {
                return parts;
            }
        }
        i += 1;
    }
    Vec::new()
}

/// Missing components count as zero: 3.12 > 3.9, 3.12.1 > 3.12.
pub fn compare_versions(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// The first line a program prints for its version flag. Python 3.9 and
/// older print -V to stderr; both streams are read.
fn version_banner(program: &str, flag: &str) -> Option<String> {
    let mut command = Command::new(program);
    command.arg(flag);
    match run_with_timeout(command, Duration::from_secs(5)) {
        Outcome::Passed(text) | Outcome::Failed(text) => text.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()),
        Outcome::Unavailable => None,
    }
}

// MARK: - Running a parser

enum Outcome {
    /// It ran and said yes, with this output.
    Passed(String),
    /// It ran and said no: its output, cut to 600 characters.
    Failed(String),
    /// It could not run, or took too long.
    Unavailable,
}

/// Runs the command with both outputs collected, killed after `timeout`.
fn run_with_timeout(mut command: Command, timeout: Duration) -> Outcome {
    let (reader, writer) = match std::io::pipe() {
        Ok(pair) => pair,
        Err(_) => return Outcome::Unavailable,
    };
    let Ok(writer_err) = writer.try_clone() else { return Outcome::Unavailable };
    command.stdin(Stdio::null()).stdout(writer).stderr(writer_err);
    let Ok(mut child) = command.spawn() else { return Outcome::Unavailable };
    drop(command); // the parent's copies of the pipe's writing end
                   // Read while it runs: a parser with a lot to say must not block on a full pipe.
    let collector = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = { reader }.read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::Unavailable;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return Outcome::Unavailable,
        }
    };
    let text = collector.join().unwrap_or_default();
    if status.success() {
        Outcome::Passed(text)
    } else {
        Outcome::Failed(text.trim().chars().take(600).collect())
    }
}

/// A file in the temporary folder, removed when dropped.
struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(extension: &str, content: &str) -> Option<TempFile> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rak-check-{}-{n}.{extension}", std::process::id()));
        std::fs::write(&path, content).ok()?;
        Some(TempFile(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available(extension: &str) -> bool {
        parser_for(extension).and_then(|p| resolve(p.names, p.version)).is_some()
    }

    #[test]
    fn leaked_diff_markers_are_transit_corruption() {
        let leaked = "+import os\n+import sys\n+\n+def main():\n+    print('hi')\nmain()\n";
        let rejection = check(Path::new("a.py"), leaked).unwrap();
        assert!(matches!(rejection, Rejection::Corrupted(_)));
        assert!(rejection.guidance().contains("resend"));
        // A few + lines in code, or any in a non-code file, are fine.
        assert_eq!(check(Path::new("notes.md"), leaked), None);
        assert_eq!(check(Path::new("a.txt"), "+1\n+2\n+3\n+4\n+5\n"), None);
    }

    #[test]
    fn json_is_parsed_and_comment_friendly_files_may_have_comments() {
        assert_eq!(check(Path::new("package.json"), r#"{ "name": "x" }"#), None);
        let broken = check(Path::new("package.json"), r#"{ "name": "x", }"#).unwrap();
        assert!(matches!(&broken, Rejection::Syntax { judged_by, .. } if judged_by == "the JSON parser"));
        assert!(broken.guidance().contains("NOT transit corruption"));
        let with_comments = "{\n  // compiler options\n  \"strict\": true, /* yes */\n  \"paths\": [\"a\",],\n}\n";
        assert_eq!(check(Path::new("tsconfig.json"), with_comments), None);
        assert_eq!(check(Path::new("proj/.vscode/settings.json"), with_comments), None);
        assert!(check(Path::new("data.json"), with_comments).is_some());
        assert!(check(Path::new("tsconfig.json"), "{ \"a\": }").is_some(), "comments allowed, broken JSON still refused");
    }

    #[test]
    fn comments_inside_strings_are_kept() {
        assert_eq!(strip_json_comments(r#"{"url": "http://x//y", "t": "a /* b */"} // c"#), r#"{"url": "http://x//y", "t": "a /* b */"} "#);
        assert_eq!(strip_json_comments(r#"["a\"", 1,]"#), r#"["a\"", 1]"#);
    }

    #[test]
    fn python_is_judged_by_its_own_parser() {
        if !available("py") {
            return;
        }
        assert_eq!(check(Path::new("ok.py"), "def f(x):\n    return x + 1\n"), None);
        let rejection = check(Path::new("/work/bad.py"), "def f(x)\n    return x\n").unwrap();
        let Rejection::Syntax { reason, judged_by } = &rejection else { panic!("{rejection:?}") };
        assert!(judged_by.contains("python3") && judged_by.contains("Python 3."), "{judged_by}");
        assert!(reason.contains("SyntaxError"), "{reason}");
        assert!(reason.contains("/work/bad.py") && !reason.contains("rak-check-"), "the model sees its own path: {reason}");
    }

    #[test]
    fn shell_and_javascript_are_checked_when_their_programs_exist() {
        if available("sh") {
            assert_eq!(check(Path::new("run.sh"), "if true; then echo hi; fi\n"), None);
            assert!(check(Path::new("run.sh"), "if true; then echo hi\n").is_some());
        }
        if available("js") {
            assert_eq!(check(Path::new("a.js"), "const x = 1;\n"), None);
            assert!(check(Path::new("a.js"), "const x = ;\n").is_some());
        }
    }

    #[test]
    fn unknown_kinds_and_huge_files_pass_unjudged() {
        assert_eq!(check(Path::new("a.zig"), "this is not zig"), None);
        assert_eq!(check(Path::new("Makefile"), "anything"), None);
        let huge = format!("def f(:\n{}", "x = 1\n".repeat(MAX_CHECKED_BYTES / 6 + 1));
        assert_eq!(check(Path::new("big.py"), &huge), None);
    }

    #[test]
    fn versions_are_read_and_compared() {
        assert_eq!(parse_version("Python 3.12.8"), [3, 12, 8]);
        assert_eq!(parse_version("v22.3.0"), [22, 3, 0]);
        assert_eq!(parse_version("Dart SDK version: 3.5.0 (stable)"), [3, 5, 0]);
        assert_eq!(parse_version("GNU bash, version 5.2.37(1)-release"), [5, 2, 37]);
        assert!(parse_version("no version").is_empty());
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions(&[3, 12], &[3, 9]), Greater);
        assert_eq!(compare_versions(&[3, 12, 1], &[3, 12]), Greater);
        assert_eq!(compare_versions(&[3, 12, 0], &[3, 12]), Equal);
    }
}
