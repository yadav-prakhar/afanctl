//! L2 death path (PRD R4): pre-opened `O_WRONLY` fd on `fan1_manual`, a panic
//! hook plus raw `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers that perform
//! exactly one async-signal-safe `write(fd, b"0")` — restoring AUTO and the
//! firmware net in a single syscall.
//!
//! The ONLY module allowed `unsafe` (§8). Every unsafe block carries a
//! `// SAFETY:` comment naming the async-signal-safety argument: no
//! allocation, no formatting, no locks, no path construction inside handlers.

/// Pre-open fd + install panic hook and raw SIGSEGV/SIGABRT/SIGTERM/SIGINT
/// handlers that write b"0" (AUTO) to the fan manual file in a single
/// write(2). Async-signal-safe only.
pub fn install_death_path(panic_fd: i32) {
    let _ = panic_fd;
    unimplemented!("owned by T4")
}

/// Deliberate panic for `afanctl selftest-panic` (proves L2; verified by
/// fixture/hw tests). Must be a `panic!`, not an abort, so the hook runs.
pub fn arm_test_panic() -> ! {
    unimplemented!("owned by T4")
}
