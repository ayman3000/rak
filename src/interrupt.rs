//! The stop flag: set when the person asks to stop a request (Ctrl-C). The
//! agent loop, the model stream and the shell tool all look at the same one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    pub fn new() -> Self {
        Stop::default()
    }
    pub fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    /// Before each request: an old Ctrl-C must not stop the next one.
    pub fn clear(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
