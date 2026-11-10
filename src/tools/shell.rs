//! run_shell: a real shell command, with a time limit that kills the whole
//! process tree, and a background mode for servers.
//!
//! The command runs in its own process group so a timeout can kill everything
//! it started. The run ends when the shell exits, not when its output pipe
//! closes: a child the command put in the background keeps the pipe open for
//! as long as it lives, and waiting for that would hold the agent until the
//! timeout and then kill the server it had just started.

use super::{bool_arg, int_arg, str_arg, Ctx, Tool, ToolResult};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_TIMEOUT_SECONDS: u64 = 900;

pub struct RunShell {
    ctx: Arc<Ctx>,
    pub max_output_chars: usize,
    pub default_timeout: Duration,
    /// How long a background command is watched before returning, to catch a
    /// server's "listening on…" line.
    pub background_grace: Duration,
}

impl RunShell {
    pub fn new(ctx: Arc<Ctx>) -> Self {
        RunShell {
            ctx,
            max_output_chars: 10_000,
            default_timeout: Duration::from_secs(120),
            background_grace: Duration::from_millis(2_500),
        }
    }
}

impl Tool for RunShell {
    fn name(&self) -> &'static str {
        "run_shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command and return its combined stdout/stderr and exit code. Use for terminal tasks \
         (git, builds, tests, installing packages). Requires approval. Optionally set `working_directory`. \
         For long-lived servers (e.g. `npm start`, `vite`, `python -m http.server`) set `background: true` — \
         the command keeps running and the tool returns immediately with its pid and any startup output. \
         Foreground commands are killed after `timeout_seconds` (default 120); for builds and test suites \
         pass a larger `timeout_seconds` (up to 900)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to run, e.g. `ls -la`." },
                "working_directory": { "type": "string", "description": "Directory to run in (a leading ~ is expanded)." },
                "background": { "type": "boolean", "description": "Run detached for long-lived servers; returns immediately with the pid instead of waiting for exit." },
                "timeout_seconds": { "type": "integer", "description": "Seconds to allow before the command is killed. Default 120; up to 900 for builds and test suites." }
            },
            "required": ["command"]
        })
    }
    fn needs_approval(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> ToolResult {
        let Some(command) = str_arg(args, "command").map(str::trim).filter(|s| !s.is_empty()) else {
            return ToolResult::err("run_shell requires a non-empty `command`.");
        };
        let cwd = match str_arg(args, "working_directory").filter(|s| !s.is_empty()) {
            Some(raw) => self.ctx.resolve(raw),
            None => self.ctx.cwd.clone(),
        };
        if !cwd.is_dir() {
            return ToolResult::err(format!("working_directory is not a folder: {}", cwd.display()));
        }
        let background = bool_arg(args, "background").unwrap_or_else(|| looks_like_long_lived_server(command));
        if background {
            self.run_background(command, &cwd)
        } else {
            self.run_foreground(command, &cwd, effective_timeout(int_arg(args, "timeout_seconds"), self.default_timeout))
        }
    }
}

/// A per-call `timeout_seconds`, clamped to 1…900, or the default.
pub fn effective_timeout(requested: Option<i64>, default: Duration) -> Duration {
    match requested {
        Some(n) => Duration::from_secs((n.max(1) as u64).min(MAX_TIMEOUT_SECONDS)),
        None => default,
    }
}

/// Commands that never exit on their own. Only whole-command prefixes: a
/// `grep vite package.json` must not be mistaken for a server.
pub fn looks_like_long_lived_server(command: &str) -> bool {
    const SERVERS: [&str; 14] = [
        "npm start", "npm run dev", "npm run serve", "yarn dev", "yarn start", "pnpm dev", "ng serve", "npx vite",
        "vite", "python -m http.server", "python3 -m http.server", "flask run", "uvicorn ", "rails server",
    ];
    let c = command.trim();
    !c.ends_with('&') && !c.contains("&&") && !c.contains('|') && SERVERS.iter().any(|s| c == s.trim() || c.starts_with(&format!("{} ", s.trim())))
}

fn shell_command(command: &str, cwd: &Path) -> Command {
    #[cfg(unix)]
    let mut cmd = {
        use std::os::unix::process::CommandExt;
        let shell = ["/bin/bash", "/usr/bin/bash"].into_iter().find(|p| Path::new(p).exists()).unwrap_or("/bin/sh");
        let mut cmd = Command::new(shell);
        cmd.arg("-c").arg(command);
        cmd.process_group(0); // its own group, so the whole tree can be killed
        cmd
    };
    #[cfg(not(unix))]
    let mut cmd = {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    };
    cmd.current_dir(cwd).stdin(Stdio::null());
    cmd
}

fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.kill();
}

fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    status.code().unwrap_or(-1)
}

impl RunShell {
    fn run_foreground(&self, command: &str, cwd: &Path, timeout: Duration) -> ToolResult {
        let (mut reader, writer) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => return ToolResult::err(format!("Failed to launch shell: {e}")),
        };
        let mut cmd = shell_command(command, cwd);
        match writer.try_clone() {
            Ok(copy) => cmd.stdout(copy).stderr(writer),
            Err(e) => return ToolResult::err(format!("Failed to launch shell: {e}")),
        };
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Failed to launch shell: {e}")),
        };
        drop(cmd); // closes our copies of the pipe's write end

        // Output is collected on its own thread, so a command that prints a
        // lot never blocks on a full pipe.
        let keep_bytes = self.max_output_chars * 4;
        let collected = Arc::new(Mutex::new(Vec::<u8>::new()));
        let closed = Arc::new(AtomicBool::new(false));
        {
            let (collected, closed) = (collected.clone(), closed.clone());
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut all = collected.lock().unwrap();
                            let room = keep_bytes.saturating_sub(all.len());
                            all.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
                closed.store(true, Ordering::SeqCst);
            });
        }

        let started = Instant::now();
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= timeout => {
                    timed_out = true;
                    kill_tree(&mut child);
                    match child.wait() {
                        Ok(status) => break status,
                        Err(e) => return ToolResult::err(format!("Lost track of the command: {e}")),
                    }
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(if started.elapsed() < Duration::from_millis(200) { 2 } else { 15 })),
                Err(e) => return ToolResult::err(format!("Lost track of the command: {e}")),
            }
        };

        // Output written just before the shell exited may still be in flight.
        // Past a short wait, whatever still holds the pipe is a background
        // child and is left running.
        let drain_until = Instant::now() + Duration::from_millis(if timed_out { 500 } else { 300 });
        while !closed.load(Ordering::SeqCst) && Instant::now() < drain_until {
            std::thread::sleep(Duration::from_millis(5));
        }
        let pipe_closed = closed.load(Ordering::SeqCst);
        let mut output = String::from_utf8_lossy(&collected.lock().unwrap()).into_owned();

        let code = exit_code(status);
        let header = if timed_out {
            format!(
                "exit {code}. Killed after {} s (run_shell timeout). The output below is everything captured \
                 before the kill — the command may or may not have finished its work. For builds and test \
                 suites pass timeout_seconds (up to {MAX_TIMEOUT_SECONDS}), or use background: true for servers.",
                timeout.as_secs()
            )
        } else if !pipe_closed {
            format!(
                "exit {code}. Processes this command started in the background are still running; their \
                 later output is not captured (redirect it to a file to keep it)."
            )
        } else {
            format!("exit {code}")
        };
        if output.chars().count() > self.max_output_chars {
            output = output.chars().take(self.max_output_chars).collect::<String>() + "\n… [output truncated]";
        }
        ToolResult::ok(if output.is_empty() { header } else { format!("{header}\n{output}") })
    }

    /// Start a long-lived command with its output going to a log file (a full
    /// pipe can then never block the server), watch it briefly, and return
    /// with its pid and the log's path, leaving it running.
    fn run_background(&self, command: &str, cwd: &Path) -> ToolResult {
        static NEXT_LOG: AtomicU32 = AtomicU32::new(0);
        let log_path: PathBuf = std::env::temp_dir().join(format!(
            "run_shell-{}-{}.log",
            std::process::id(),
            NEXT_LOG.fetch_add(1, Ordering::SeqCst)
        ));
        let log = match std::fs::File::create(&log_path).and_then(|f| Ok((f.try_clone()?, f))) {
            Ok(pair) => pair,
            Err(e) => return ToolResult::err(format!("Failed to open background log file: {e}")),
        };
        let mut cmd = shell_command(command, cwd);
        cmd.stdout(log.0).stderr(log.1);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Failed to launch shell: {e}")),
        };
        drop(cmd);

        std::thread::sleep(self.background_grace);
        let text = String::from_utf8_lossy(&std::fs::read(&log_path).unwrap_or_default()).into_owned();
        let count = text.chars().count();
        let early = if count > self.max_output_chars {
            text.chars().skip(count - self.max_output_chars).collect::<String>() + "\n… [truncated]"
        } else {
            text
        };

        if let Ok(Some(status)) = child.try_wait() {
            let header = format!(
                "exit {} (background command finished within {}s)",
                exit_code(status),
                self.background_grace.as_secs()
            );
            return ToolResult::ok(if early.is_empty() { header } else { format!("{header}\n{early}") });
        }
        let mut header = format!(
            "started in background (pid {}); still running. Full output: {}",
            child.id(),
            log_path.display()
        );
        if !early.is_empty() {
            header.push_str(&format!("\n--- startup output ---\n{early}"));
        }
        ToolResult::ok(header)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::tools::testing::temp_dir;

    fn shell(label: &str) -> RunShell {
        RunShell::new(Ctx::new(&temp_dir(label), false))
    }

    #[test]
    fn output_and_exit_code_come_back_with_stderr_included() {
        let r = shell("out").run(&json!({ "command": "echo out; echo err 1>&2; exit 3" }));
        assert!(r.ok);
        assert!(r.text.starts_with("exit 3\n"), "{}", r.text);
        assert!(r.text.contains("out") && r.text.contains("err"));
    }

    #[test]
    fn runs_in_the_project_folder_by_default() {
        let s = shell("cwd");
        let r = s.run(&json!({ "command": "pwd" }));
        assert_eq!(r.text, format!("exit 0\n{}\n", s.ctx.cwd.display()));
    }

    #[test]
    fn a_command_past_its_timeout_is_killed_with_what_it_printed() {
        let started = Instant::now();
        let r = shell("timeout").run(&json!({ "command": "echo begun; sleep 30", "timeout_seconds": 1 }));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(r.text.contains("Killed after 1 s") && r.text.contains("begun"), "{}", r.text);
    }

    #[test]
    fn the_timeout_kills_grandchildren_too() {
        let s = shell("tree");
        let marker = s.ctx.cwd.join("alive");
        s.run(&json!({ "command": "(sleep 2; touch alive) & sleep 30", "timeout_seconds": 1 }));
        std::thread::sleep(Duration::from_millis(2_500));
        assert!(!marker.exists(), "the grandchild outlived the kill");
    }

    #[test]
    fn a_child_left_in_the_background_does_not_hold_the_run() {
        let started = Instant::now();
        let r = shell("bg-child").run(&json!({ "command": "sleep 20 & echo started" }));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(r.text.contains("started") && r.text.contains("still running"), "{}", r.text);
    }

    #[test]
    fn background_mode_returns_with_pid_and_startup_output() {
        let mut s = shell("background");
        s.background_grace = Duration::from_millis(400);
        let r = s.run(&json!({ "command": "echo listening; sleep 20", "background": true }));
        assert!(r.text.starts_with("started in background (pid "), "{}", r.text);
        assert!(r.text.contains("listening"));
    }

    #[test]
    fn long_output_is_cut_off() {
        let mut s = shell("long");
        s.max_output_chars = 50;
        let r = s.run(&json!({ "command": "yes line | head -n 500" }));
        assert!(r.text.ends_with("… [output truncated]"), "{}", r.text);
        assert!(r.text.len() < 200);
    }

    #[test]
    fn servers_are_spotted_but_lookalikes_are_not() {
        assert!(looks_like_long_lived_server("npm run dev"));
        assert!(looks_like_long_lived_server("python3 -m http.server 8080"));
        assert!(looks_like_long_lived_server("vite"));
        assert!(!looks_like_long_lived_server("grep vite package.json"));
        assert!(!looks_like_long_lived_server("npm start && echo done"));
        assert!(!looks_like_long_lived_server("npm test"));
        assert!(!looks_like_long_lived_server("vitest run"));
    }

    #[test]
    fn timeouts_are_clamped() {
        let d = Duration::from_secs(120);
        assert_eq!(effective_timeout(None, d), d);
        assert_eq!(effective_timeout(Some(5_000), d), Duration::from_secs(900));
        assert_eq!(effective_timeout(Some(-4), d), Duration::from_secs(1));
    }
}
