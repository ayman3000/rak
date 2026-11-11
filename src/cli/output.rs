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
pub fn describe(name: &str, args: &Value) -> String {
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let lines = |key: &str| text(key).split('\n').count();
    match name {
        "edit_file" => format!("edit_file {}  ({} → {} lines)", text("path"), lines("old_text"), lines("new_text")),
        "write_file" => format!("write_file {}  ({} bytes)", text("path"), text("content").len()),
        "run_shell" => format!("run_shell {}", one_line(text("command"), 110)),
        _ => {
            let mut parts: Vec<String> = args
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| format!("{k}={}", one_line(&v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()), 60)))
                .collect();
            parts.sort();
            format!("{name} {}", parts.join(" "))
        }
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
                } else {
                    let line = format!("  {} {}", if *ok { "✓" } else { "✗" }, one_line(text, 110));
                    ui.progress(Some(if *ok { Style::Dim } else { Style::Red }), &fit(&line));
                }
            }
            Event::Note(note) => {
                log.write(json!({ "event": "note", "note": note }));
                ui.progress(Some(Style::Dim), &format!("  {note}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_described_briefly() {
        let edit = json!({ "path": "a.rs", "old_text": "x\ny\nz", "new_text": "x" });
        assert_eq!(describe("edit_file", &edit), "edit_file a.rs  (3 → 1 lines)");
        assert_eq!(describe("write_file", &json!({ "path": "b", "content": "hello" })), "write_file b  (5 bytes)");
        assert_eq!(describe("run_shell", &json!({ "command": "cargo  test\n--quiet" })), "run_shell cargo test --quiet");
        assert_eq!(describe("files__search", &json!({ "q": "x", "limit": 3 })), "files__search limit=3 q=x");
    }
}
