//! L2 death path (PRD R4): pre-opened `O_WRONLY` fd on `fan1_manual`, a panic
//! hook plus raw `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers that perform
//! exactly one async-signal-safe `write(fd, b"0")` — restoring AUTO and the
//! firmware net in a single syscall.
//!
//! The ONLY module allowed `unsafe` (§8). Every unsafe block carries a
//! `// SAFETY:` comment naming the async-signal-safety argument: no
//! allocation, no formatting, no locks, no path construction inside handlers.

#![allow(clippy::panic)] // arm_test_panic is the sanctioned deliberate panic (§8)

use std::panic;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Armed fd, `-1` = unarmed. Lock-free atomic: loadable from a raw signal
/// handler with no locks, no allocation.
static PANIC_FD: AtomicI32 = AtomicI32::new(-1);
/// Guards against double-hooking if install is called more than once.
static HOOKED: AtomicBool = AtomicBool::new(false);
/// The single byte written to the manual file: `b"0"` = AUTO.
static AUTO: [u8; 1] = *b"0";

/// Pre-open fd + install panic hook and raw SIGSEGV/SIGABRT/SIGTERM/SIGINT
/// handlers that write b"0" (AUTO) to the fan manual file in a single
/// write(2). Async-signal-safe only.
pub fn install_death_path(panic_fd: i32) {
    PANIC_FD.store(panic_fd, Ordering::SeqCst);
    if !HOOKED.swap(true, Ordering::SeqCst) {
        let prev = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            write_auto();
            prev(info); // keep standard panic reporting (hook is not a signal handler)
        }));
    }
    // SAFETY: install-time only, single-threaded (§8) and before any signal is
    // delivered to this process; `sigaction` only writes the kernel's
    // disposition table and cannot fail on these well-formed inputs.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = libc::SA_RESTART;
        sa.sa_sigaction = on_signal as *const () as usize;
        for sig in [libc::SIGSEGV, libc::SIGABRT, libc::SIGTERM, libc::SIGINT] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// The death write itself: exactly one `write(fd, b"0", 1)`, nothing else.
/// Shared by the panic hook and the raw handlers.
fn write_auto() {
    // SAFETY: async-signal-safe — `write(2)` is on POSIX's handler-safe list;
    // the fd comes from one lock-free atomic load; the buffer is a static;
    // no allocation, no formatting, no locks, no path construction. A failed
    // or short write is deliberately ignored: L2 is best-effort by design
    // (the firmware net and L3 Restart=always are the remaining layers).
    unsafe {
        let fd = PANIC_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            libc::write(fd, AUTO.as_ptr().cast(), 1);
        }
    }
}

extern "C" fn on_signal(sig: libc::c_int) {
    write_auto();
    // SAFETY: re-raise pattern so the process dies with the original signal
    // exactly once — a SIGSEGV handler that merely returns would resume the
    // faulting instruction and re-enter the handler (re-writing the fan file
    // forever). `signal` and `raise` are both on POSIX's async-signal-safe
    // list; no allocation, locks, or formatting here.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

/// Deliberate panic for `afanctl selftest-panic` (proves L2; verified by
/// fixture/hw tests). Must be a `panic!`, not an abort, so the hook runs.
pub fn arm_test_panic() -> ! {
    panic!("afanctl selftest-panic: deliberate L2 death-path probe")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // T9-F4: tests are allowlisted (§8)

    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Proves the panic-hook half of L2: after install, a panic writes `b"0"`
    /// to the armed fd; an unarmed (`-1`) fd skips the write. One test fn:
    /// `install_death_path` mutates process-global state (hook + fd). The raw
    /// signal half is proven by T8's `selftest-panic` fixture test + hw gate.
    #[test]
    fn l2_panic_hook_writes_auto_and_negative_fd_is_ignored() {
        let path = std::env::temp_dir().join(format!("afanctl-t4-l2-{}", std::process::id()));
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        install_death_path(f.as_raw_fd());
        let payload = catch_unwind(AssertUnwindSafe(|| arm_test_panic())).unwrap_err();
        let msg = payload.downcast_ref::<&str>().unwrap_or(&"");
        assert!(msg.contains("selftest-panic"), "unexpected payload: {msg}");
        assert_eq!(fs::read(&path).unwrap(), b"0", "hook must write AUTO byte");

        install_death_path(-1); // disarm: handler/hook must skip the write
        let _ = catch_unwind(AssertUnwindSafe(|| arm_test_panic()));
        assert_eq!(
            fs::read(&path).unwrap(),
            b"0",
            "unarmed fd must not be written"
        );

        let _ = fs::remove_file(&path);
    }
}
