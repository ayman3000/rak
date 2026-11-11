//! The terminal itself: its size, raw mode, and reading keys a byte at a time.

/// (columns, rows); 80 × 24 when stdout isn't a terminal.
#[cfg(unix)]
pub fn size() -> (usize, usize) {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_col > 0 && size.ws_row > 0 {
        (size.ws_col as usize, size.ws_row as usize)
    } else {
        (80, 24)
    }
}

#[cfg(not(unix))]
pub fn size() -> (usize, usize) {
    (80, 24)
}

/// Raw mode for as long as this value lives: keys arrive one at a time, not
/// echoed, and Ctrl-C is a key rather than a signal.
#[cfg(unix)]
pub struct Raw {
    original: libc::termios,
}

#[cfg(unix)]
impl Raw {
    pub fn enter() -> Option<Raw> {
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                return None;
            }
            let mut raw = original;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::INLCR);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &raw) != 0 {
                return None;
            }
            Some(Raw { original })
        }
    }
}

#[cfg(unix)]
impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &self.original);
        }
    }
}

#[cfg(not(unix))]
pub struct Raw;

#[cfg(not(unix))]
impl Raw {
    /// No raw mode here yet: the editor falls back to plain line input.
    pub fn enter() -> Option<Raw> {
        None
    }
}

/// One byte from the keyboard. With `wait_ms`, None when nothing arrives in
/// that time; without, waits. None also at end of input.
#[cfg(unix)]
pub fn read_byte(wait_ms: Option<i32>) -> Option<u8> {
    if let Some(ms) = wait_ms {
        let mut fds = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut fds, 1, ms) } <= 0 {
            return None;
        }
    }
    let mut byte = 0u8;
    loop {
        let n = unsafe { libc::read(libc::STDIN_FILENO, &mut byte as *mut u8 as *mut libc::c_void, 1) };
        if n == 1 {
            return Some(byte);
        }
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return None;
    }
}

#[cfg(not(unix))]
pub fn read_byte(_wait_ms: Option<i32>) -> Option<u8> {
    None
}
