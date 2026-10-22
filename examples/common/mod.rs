//! What the examples share. A directory with no `main.rs`, so cargo doesn't take it for an example.

use std::time::Duration;

/// This thread's CPU time.
pub fn cpu() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}
