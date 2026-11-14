//! What a run shows: the answer as it arrives, and a line for each step.

use super::log::Log;
use super::markdown;
use super::style::{cut, paint, Style};
use super::term;
use rak::agent::Event;
use rak::text::one_line;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

/// `edit_file src/a.rs (3 → 5 lines)`: enough to follow along, never the
/// whole text of an edit.
/// The project folder, for showing paths in it relative to it.
static PROJECT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Set once, when the run starts.
pub fn set_project(folder: &std::path::Path) {
    let _ = PROJECT.set(folder.to_string_lossy().trim_end_matches('/').to_string());
}

/// Pure — unit-tested. A path inside `project` shown relative to it, so the
/// part that tells files apart is not cut off; anything else as it is.
pub fn relative_to(project: &str, value: &str) -> String {
    match value.strip_prefix(project) {
        Some("") => ".".to_string(),
        Some(rest) if rest.starts_with('/') => rest.trim_start_matches('/').to_string(),
        _ => value.to_string(),
    }
}

fn shown(value: &str) -> String {
    match PROJECT.get() {
        Some(project) if !project.is_empty() => relative_to(project, value),
        _ => value.to_string(),
    }
}

pub fn describe(name: &str, args: &Value) -> String {
    let path_text = |key: &str| shown(args.get(key).and_then(Value::as_str).unwrap_or(""));
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let lines = |key: &str| text(key).split('\n').count();
    match name {
        "edit_file" => format!("edit_file {}  ({} → {} lines)", path_text("path"), lines("old_text"), lines("new_text")),
        "write_file" => format!("write_file {}  ({} bytes)", path_text("path"), text("content").len()),
        "apply_patch" => match rak::tools::patch::parse(text("patch")) {
            Some(hunks) => {
                let (n, added, removed) = rak::tools::patch::counts(&hunks);
                format!("apply_patch {}  ({n} hunk{}, +{added}/-{removed})", path_text("path"), if n == 1 { "" } else { "s" })
            }
            None => format!("apply_patch {}", path_text("path")),
        },
        "run_shell" => format!("run_shell {}", one_line(text("command"), 110)),
        "delegate" => format!("delegate  {}", one_line(text("task"), 90)),
        "todo" => {
            let steps = args.get("steps").and_then(Value::as_array).map_or(0, Vec::len);
            let done = args.get("steps").and_then(Value::as_array).map_or(0, |s| s.iter().filter(|s| s["status"] == "done").count());
            format!("todo  ({done}/{steps} done)")
        }
        "save_skill" => format!("save_skill \"{}\"  ({} lines)", one_line(text("name"), 60), lines("instructions")),
        _ => {
            let mut parts: Vec<String> = args
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| format!("{k}={}", one_line(&v.as_str().map(shown).unwrap_or_else(|| v.to_string()), 60)))
                .collect();
            parts.sort();
            format!("{name} {}", parts.join(" "))
        }
    }
}

/// Pure — unit-tested. The lines shown when rak moves to another model:
/// what happens now, short enough not to be cut, then the error on its own
/// line, unless the person was just asked with that error in the question.
pub fn fallback_note(to: &str, why: &str, failure: rak::fallback::Failure, asked: bool) -> Vec<String> {
    use rak::fallback::Failure;
    let now = match failure {
        Failure::Midway => format!("doing this step again with {to}"),
        _ => format!("now using {to}"),
    };
    let mut lines = vec![now];
    if !asked {
        lines.push(one_line(why, 200));
    }
    lines
}

/// Pure — unit-tested. "1 turn", "2 turns".
pub fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Pure — unit-tested. "3 tool calls", or "3 tool calls (+14 in helpers)"
/// when delegate's helpers made calls of their own.
pub fn calls(own: usize, helpers: usize) -> String {
    let own = plural(own, "tool call");
    if helpers == 0 {
        own
    } else {
        format!("{own} (+{helpers} in helpers)")
    }
}

/// What is on screen, shared by everything that prints during a request.
pub struct Ui {
    /// Chat: progress goes to stdout with the answer. One request: to stderr,
    /// so `rak "…" > answer.txt` holds only the answer.
    pub chat: bool,
    /// Progress lines are shown at all.
    pub show: bool,
    pub out_color: bool,
    pub err_color: bool,
    /// The answer's text is printed as it arrives.
    pub live: bool,
    pub renderer: Option<markdown::Renderer>,
    /// The last text printed did not end its line.
    pub line_open: bool,
    /// The fallback question was just shown, with the error in it.
    pub asked: bool,
}

impl Ui {
    /// A piece of the answer, as it arrives.
    pub fn delta(&mut self, piece: &str) {
        if !self.live {
            return;
        }
        match &mut self.renderer {
            // Markdown is shown a whole line at a time.
            Some(renderer) => print!("{}", renderer.push(piece)),
            None => {
                print!("{piece}");
                self.line_open = !piece.ends_with('\n');
            }
        }
        let _ = std::io::stdout().flush();
    }

    /// Writes out answer text still held back and ends its line. Before a
    /// tool runs or asks, so what the model said before the call is shown
    /// before it, and at the end of the answer.
    pub fn end_line(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            let rest = renderer.finish();
            if !rest.is_empty() {
                println!("{rest}");
            }
        }
        if self.line_open {
            println!();
            self.line_open = false;
        }
        let _ = std::io::stdout().flush();
    }

    /// A progress line, styled for whichever stream it lands on.
    pub fn progress(&mut self, style: Option<Style>, text: &str) {
        if !self.show {
            return;
        }
        self.end_line();
        let styled = |color: bool| style.map(|s| paint(color, s, text)).unwrap_or_else(|| text.to_string());
        if self.chat {
            println!("{}", styled(self.out_color));
        } else {
            eprintln!("{}", styled(self.err_color));
        }
    }

    /// A whole answer that was not shown as it arrived.
    pub fn answer(&mut self, text: &str) {
        match &mut self.renderer {
            Some(_) => {
                let mut renderer = markdown::Renderer::default();
                print!("{}", renderer.push(&format!("{text}\n")));
                let rest = renderer.finish();
                if !rest.is_empty() {
                    println!("{rest}");
                }
            }
            None => println!("{text}"),
        }
    }
}

/// A progress line cut to the terminal's width, so it stays one line.
pub fn fit(line: &str) -> String {
    cut(line, term::size().0.saturating_sub(1).max(20))
}

/// What the agent's events do: written to the log, and shown.
pub fn reporter(log: Log, ui: Rc<RefCell<Ui>>) -> impl for<'a> FnMut(&Event<'a>) {
    move |event| {
        let mut ui = ui.borrow_mut();
        match event {
            Event::ModelCalled { turn, usage, millis } => {
                log.write(json!({ "event": "model_call", "turn": turn, "input_tokens": usage.input, "cached_tokens": usage.cached, "cache_write_tokens": usage.cache_write, "output_tokens": usage.output, "ms": millis }));
            }
            Event::Retrying { attempt, error } => {
                log.write(json!({ "event": "retry", "attempt": attempt, "error": error }));
                ui.progress(Some(Style::Yellow), &format!("  retrying model call ({attempt}): {}", one_line(error, 120)));
            }
            Event::Delta(piece) => ui.delta(piece),
            Event::Text(text) => {
                log.write(json!({ "event": "text", "text": text }));
                if !ui.live {
                    ui.progress(None, text.trim());
                }
            }
            Event::ToolStarted { name, args } => {
                log.write(json!({ "event": "tool_call", "tool": name, "arguments": args }));
                ui.progress(Some(Style::Cyan), &fit(&format!("→ {}", describe(name, args))));
            }
            Event::ToolFinished { name, ok, denied, text, millis } => {
                log.write(json!({ "event": "tool_result", "tool": name, "ok": ok, "denied": denied, "ms": millis, "result": text }));
                if *denied {
                    ui.progress(Some(Style::Yellow), "  ✗ not allowed");
                } else if *name == "todo" && *ok {
                    // The task list is shown whole: it is the plan the person follows.
                    for line in text.lines().skip(1) {
                        ui.progress(Some(Style::Dim), &fit(&format!("    {line}")));
                    }
                } else {
                    let line = format!("  {} {}", if *ok { "✓" } else { "✗" }, one_line(text, 110));
                    ui.progress(Some(if *ok { Style::Dim } else { Style::Red }), &fit(&line));
                }
            }
            Event::Note(note) => {
                log.write(json!({ "event": "note", "note": note }));
                ui.progress(Some(Style::Dim), &format!("  {note}"));
            }
            Event::FellBack { to, why, failure } => {
                log.write(json!({ "event": "fallback", "to": to, "error": why, "reason": format!("{failure:?}") }));
                ui.end_line();
                let asked = std::mem::take(&mut ui.asked);
                for (i, line) in fallback_note(to, why, *failure, asked).iter().enumerate() {
                    let (style, mark) = if i == 0 { (Style::Yellow, "↪ ") } else { (Style::Dim, "  ") };
                    ui.progress(Some(style), &fit(&format!("{mark}{line}")));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn helper_calls_are_shown_apart() {
        assert_eq!(calls(3, 0), "3 tool calls");
        assert_eq!(calls(1, 14), "1 tool call (+14 in helpers)");
    }

    use super::*;

    #[test]
    fn steps_are_described_briefly() {
        let edit = json!({ "path": "a.rs", "old_text": "x\ny\nz", "new_text": "x" });
        assert_eq!(describe("edit_file", &edit), "edit_file a.rs  (3 → 1 lines)");
        assert_eq!(describe("write_file", &json!({ "path": "b", "content": "hello" })), "write_file b  (5 bytes)");
        assert_eq!(
            describe("apply_patch", &json!({ "path": "c", "patch": "@@ -1 +1,2 @@\n-a\n+b\n+c\n" })),
            "apply_patch c  (1 hunk, +2/-1)"
        );
        assert_eq!(describe("run_shell", &json!({ "command": "cargo  test\n--quiet" })), "run_shell cargo test --quiet");
        assert_eq!(describe("files__search", &json!({ "q": "x", "limit": 3 })), "files__search limit=3 q=x");
        assert_eq!(relative_to("/home/a/rak", "/home/a/rak/src/agent.rs"), "src/agent.rs");
        assert_eq!(relative_to("/home/a/rak", "/home/a/rak"), ".");
        assert_eq!(relative_to("/home/a/rak", "/home/a/rakish/x.rs"), "/home/a/rakish/x.rs", "a folder that only starts the same");
        assert_eq!(relative_to("/home/a/rak", "/etc/hosts"), "/etc/hosts");
        use rak::fallback::Failure;
        assert_eq!(
            fallback_note("anthropic · claude-haiku-5-5", "the model server answered 503: busy", Failure::Transient, false),
            ["now using anthropic · claude-haiku-5-5", "the model server answered 503: busy"],
            "what happens now on its own line, so a long error never cuts it"
        );
        assert_eq!(fallback_note("x · y", "connection reset", Failure::Midway, false)[0], "doing this step again with x · y");
        assert_eq!(
            fallback_note("x · y", "answered 401: bad key", Failure::Config, true),
            ["now using x · y"],
            "the question showed the error"
        );
        assert_eq!(
            (plural(1, "turn"), plural(2, "turn"), plural(0, "tool call")),
            ("1 turn".into(), "2 turns".into(), "0 tool calls".into())
        );
    }
}
