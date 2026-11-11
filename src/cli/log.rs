//! `--log FILE`: every step of a run as JSON lines.

use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;

/// Appends one JSON object per line. Errors are ignored: a log that cannot be
/// written must not stop the work.
#[derive(Clone)]
pub struct Log(Option<Rc<RefCell<std::fs::File>>>);

impl Log {
    pub fn open(path: Option<&PathBuf>) -> Log {
        let file = path.map(|p| {
            std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap_or_else(|e| {
                eprintln!("rak: cannot open the log {}: {e}", p.display());
                std::process::exit(1);
            })
        });
        Log(file.map(|f| Rc::new(RefCell::new(f))))
    }
    pub fn write(&self, mut entry: Value) {
        if let Some(file) = &self.0 {
            let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            entry["time"] = json!((seconds * 1000.0).round() / 1000.0);
            let _ = writeln!(file.borrow_mut(), "{entry}");
        }
    }
}
