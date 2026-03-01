use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Install a SIGINT handler that sets the `INTERRUPTED` flag.
///
/// Uses `sa_flags = 0` (no `SA_RESTART`) so that blocking
/// `read()` calls return `EINTR` when the signal fires.
pub fn install_handler() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as usize;
        sa.sa_flags = 0; // no SA_RESTART
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }
}

extern "C" fn handler(_sig: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn clear() {
    INTERRUPTED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn set() {
    INTERRUPTED.store(true, Ordering::SeqCst);
}
