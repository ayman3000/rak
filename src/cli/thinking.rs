//! The line shown while the model is working and nothing else is arriving:
//! `Thinking… 12s`, dim, on one line, gone the moment the answer or a tool
//! line is shown. A reply that starts within the first second never draws it.
//!
//! One thread, started the first time it is needed, asleep unless a model call
//! is being waited on; then it wakes once a second. The line is drawn and
//! erased under one lock, so it never lands inside other output. Only on a
//! terminal: never in --json, a pipe or a log.

use super::style::{paint, Style};
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A wait at least this long leaves `Thought for Ns` behind.
pub const WORTH_NOTING: Duration = Duration::from_secs(5);

#[derive(Default)]
struct State {
    /// A model call is being waited on, since then.
    since: Option<Instant>,
    /// The line is on screen.
    drawn: bool,
}

pub struct Thinking {
    shared: Arc<(Mutex<State>, Condvar)>,
    /// Stdout (a chat) or stderr (one request, whose stdout is the answer).
    to_stdout: bool,
    color: bool,
    started: bool,
}

/// Pure — unit-tested. "Thinking… 12s", "Thinking… 1m 05s".
pub fn label(word: &str, seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{word} {seconds}s"),
        _ => format!("{word} {}m {:02}s", seconds / 60, seconds % 60),
    }
}

fn write(to_stdout: bool, text: &str) {
    if to_stdout {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes()).and_then(|_| out.flush());
    } else {
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(text.as_bytes()).and_then(|_| err.flush());
    }
}

const ERASE: &str = "\r\x1b[2K";

impl Thinking {
    pub fn new(to_stdout: bool, color: bool) -> Thinking {
        Thinking { shared: Arc::default(), to_stdout, color, started: false }
    }

    /// A model call begins.
    pub fn start(&mut self) {
        if !self.started {
            self.started = true;
            let (shared, to_stdout, color) = (self.shared.clone(), self.to_stdout, self.color);
            // If the thread cannot start there is simply no line.
            let _ = std::thread::Builder::new().name("thinking".into()).spawn(move || tick(&shared, to_stdout, color));
        }
        if let Ok(mut state) = self.shared.0.lock() {
            state.since = Some(Instant::now());
            self.shared.1.notify_one();
        }
    }

    /// Something else is about to be shown, or the call ended: the line goes.
    /// How long the wait was, when the line had been drawn.
    pub fn stop(&mut self) -> Option<Duration> {
        let mut state = self.shared.0.lock().ok()?;
        let since = state.since.take()?;
        if !std::mem::take(&mut state.drawn) {
            return None;
        }
        write(self.to_stdout, ERASE);
        Some(since.elapsed())
    }
}

fn tick(shared: &(Mutex<State>, Condvar), to_stdout: bool, color: bool) {
    let Ok(mut state) = shared.0.lock() else { return };
    loop {
        let Some(since) = state.since else {
            // Asleep until a call begins.
            match shared.1.wait(state) {
                Ok(woken) => state = woken,
                Err(_) => return,
            }
            continue;
        };
        // Until the next whole second of this wait.
        let elapsed = since.elapsed();
        let next = Duration::from_secs(elapsed.as_secs() + 1) - elapsed;
        match shared.1.wait_timeout(state, next) {
            Ok((woken, _)) => state = woken,
            Err(_) => return,
        }
        if state.since == Some(since) && since.elapsed() >= Duration::from_secs(1) {
            write(to_stdout, &format!("{ERASE}{}", paint(color, Style::Dim, &label("Thinking…", since.elapsed().as_secs()))));
            state.drawn = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_is_said_in_seconds_then_minutes() {
        assert_eq!(label("Thinking…", 1), "Thinking… 1s");
        assert_eq!(label("Thinking…", 59), "Thinking… 59s");
        assert_eq!(label("Thinking…", 65), "Thinking… 1m 05s");
        assert_eq!(label("Thought for", 14), "Thought for 14s");
    }

    #[test]
    fn a_quick_reply_draws_nothing_and_a_slow_one_reports_its_wait() {
        let mut thinking = Thinking::new(false, false);
        assert_eq!(thinking.stop(), None, "nothing was being waited on");
        thinking.start();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(thinking.stop(), None, "gone before the first second: never drawn");
        thinking.start();
        std::thread::sleep(Duration::from_millis(1150));
        let waited = thinking.stop().expect("drawn after a second");
        assert!(waited >= Duration::from_secs(1) && waited < Duration::from_secs(3), "{waited:?}");
        assert_eq!(thinking.stop(), None, "stopped once");
    }
}
