//! Ctrl-C. While a request runs, the first one asks it to stop (the shared
//! stop flag); a second one, or one between requests, quits rak.

use rak::interrupt::Stop;

#[cfg(unix)]
mod unix {
    use rak::interrupt::Stop;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    static STOP: OnceLock<Stop> = OnceLock::new();
    pub static RUNNING: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle(_: libc::c_int) {
        let first = STOP.get().filter(|stop| RUNNING.load(Ordering::SeqCst) && !stop.requested());
        match first {
            Some(stop) => {
                stop.request();
                let note = "\n⏹ stopping… (Ctrl-C again quits rak)\n".as_bytes();
                unsafe { libc::write(2, note.as_ptr() as *const libc::c_void, note.len()) };
            }
            None => unsafe { libc::_exit(130) },
        }
    }

    pub fn install(stop: Stop) {
        let _ = STOP.set(stop);
        unsafe { libc::signal(libc::SIGINT, handle as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    }
}

pub fn install(_stop: Stop) {
    #[cfg(unix)]
    unix::install(_stop);
}

/// Whether a request is running: Ctrl-C stops it rather than quitting.
pub fn set_running(_on: bool) {
    #[cfg(unix)]
    unix::RUNNING.store(_on, std::sync::atomic::Ordering::SeqCst);
}
