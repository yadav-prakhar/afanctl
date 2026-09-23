//! L2 death path (PRD R4): a pre-opened `O_WRONLY` fd plus the exact bytes
//! that hand the fan back to the firmware, both supplied by the backend as a
//! [`SafeRestore`] descriptor. A panic hook and raw
//! `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers perform exactly one
//! async-signal-safe `write(2)` of those bytes.
//!
//! RULING F27: this module names **no attribute and no restore value**, not
//! even in a comment. Both differ per backend and, for applesmc, per sysfs ABI
//! generation, where the two AUTO tokens are mutually incompatible — so a
//! compile-time constant here would silently strand the fan in manual. `smc.rs`
//! owns that table (see `docs/SAFETY.md`), and the backend must also *prove*
//! its restore works — `Smc::probe_safe_restore` — before anything advertises
//! L2 as armed.
//!
//! The ONLY module allowed `unsafe` (§8). Every unsafe block carries a
//! `// SAFETY:` comment naming the async-signal-safety argument: one lock-free
//! atomic load, one `write(2)`, no allocation, no formatting, no locks, no
//! path construction inside handlers.

#![allow(clippy::panic)] // arm_test_panic is the sanctioned deliberate panic (§8)

use std::os::fd::RawFd;
use std::panic;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

/// Everything the async-signal-safe death path needs, resolved up front by
/// the backend (RULING F27).
///
/// INVARIANT: `bytes` is `'static` — a per-backend constant slice, never a
/// heap or formatted value — so the handler never touches the allocator. A
/// multi-byte restore (thinkpad_acpi's `"level auto"`) is still a single
/// `write(2)`; its length is simply part of the descriptor.
#[derive(Debug, Clone, Copy)]
pub struct SafeRestore {
    /// Pre-opened `O_WRONLY` fd on the file that returns control to firmware.
    /// A negative fd is the disarmed descriptor: the handler writes nothing.
    fd: RawFd,
    /// The exact bytes to write. One `write(2)`, no formatting.
    bytes: &'static [u8],
}

impl SafeRestore {
    /// Build a descriptor. The backend chooses both halves; this module only
    /// stores them and writes them (R1: no attribute name or value here).
    pub fn new(fd: RawFd, bytes: &'static [u8]) -> Self {
        Self { fd, bytes }
    }

    /// The pre-opened fd the handler writes to.
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// The exact restore payload — what a test asserts the death path writes.
    pub fn bytes(&self) -> &'static [u8] {
        self.bytes
    }
}

/// Which safety layers a backend can offer beyond L1 (verified writes) and L2
/// (this death path). Published in `state.json` / `status --json` so the
/// plugin stops inferring one universal mechanism from a single boolean
/// (RULING F27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyCapabilities {
    /// Stable backend id, including the bound ABI generation where one
    /// applies (e.g. `applesmc/modern`). Names no sysfs attribute (R1).
    pub backend: &'static str,
    /// True when the backend exposes a firmware/EC watchdog that keeps
    /// working after `SIGKILL` — the one layer no signal handler can provide.
    pub hw_watchdog: bool,
    /// `Some(true)` when the firmware is documented to return the fan to auto
    /// across suspend, `Some(false)` when documented not to, `None` when
    /// unproven for this backend. Never asserted without evidence.
    pub firmware_auto_on_suspend: Option<bool>,
}

/// The armed descriptor in leaked `'static` storage; null = unarmed. A raw
/// signal handler reaches it with ONE lock-free atomic load.
static ARMED: AtomicPtr<SafeRestore> = AtomicPtr::new(std::ptr::null_mut());
/// Guards against double-hooking if install is called more than once.
static HOOKED: AtomicBool = AtomicBool::new(false);

/// Arm the death path with a backend-supplied descriptor and install the panic
/// hook plus raw SIGSEGV/SIGABRT/SIGTERM/SIGINT handlers, which write its
/// bytes to its fd in a single `write(2)`. Async-signal-safe only.
///
/// The caller must have proven the restore first (`Smc::probe_safe_restore`,
/// RULING F27): this path ignores write errors by design, so an unproven
/// descriptor would advertise a layer that does nothing.
pub fn install_death_path(restore: SafeRestore) {
    // INVARIANT (async-signal-safety): the descriptor is published into
    // `'static` storage — deliberately leaked, once per arm call, at arm time
    // only — so the handler dereferences memory that is never freed, never
    // mutated after publication, and needs no allocation of its own. That is
    // what keeps the handler at exactly one atomic load.
    ARMED.store(
        Box::leak(Box::new(restore)) as *mut SafeRestore,
        Ordering::SeqCst,
    );
    if !HOOKED.swap(true, Ordering::SeqCst) {
        let prev = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            write_restore();
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

/// True when a *usable* descriptor is armed — the single source of truth for
/// "L2 really will write on death", whoever armed it. One lock-free atomic
/// load, so `state.json` publishing can ask it every poll (RULING F27).
pub fn is_armed() -> bool {
    // SAFETY: one atomic load of a pointer into leaked `'static` storage that
    // is never freed and never mutated after publication, so the reference is
    // valid for as long as it is held. No allocation, no locks.
    unsafe {
        let armed = ARMED.load(Ordering::SeqCst);
        !armed.is_null() && (*armed).fd >= 0
    }
}

/// The death write itself: exactly one `write(fd, bytes, len)`, nothing else.
/// Shared by the panic hook and the raw handlers.
fn write_restore() {
    // SAFETY: async-signal-safe — `write(2)` is on POSIX's handler-safe list;
    // the descriptor comes from ONE lock-free atomic load of a pointer into
    // leaked `'static` storage that is never freed or mutated after
    // publication; the payload is the backend's `'static` slice. No
    // allocation, no formatting, no locks, no path construction. A failed or
    // short write is deliberately ignored: L2 is best-effort by design (the
    // firmware net and L3 `Restart=always` are the remaining layers) — which
    // is exactly why the descriptor is proven at arm time (RULING F27)
    // instead of trusted here.
    unsafe {
        let armed = ARMED.load(Ordering::SeqCst);
        if armed.is_null() {
            return;
        }
        let restore = &*armed;
        if restore.fd >= 0 {
            libc::write(
                restore.fd,
                restore.bytes.as_ptr().cast(),
                restore.bytes.len(),
            );
        }
    }
}

extern "C" fn on_signal(sig: libc::c_int) {
    write_restore();
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

    /// A deliberately multi-byte payload: RULING F27 makes the length part of
    /// the descriptor, so one `write(2)` must still carry all of it. No
    /// single-byte assumption may creep back into this module.
    static RESTORE: &[u8] = b"level auto";

    /// Proves the panic-hook half of L2: after arming with a backend
    /// descriptor, a panic writes exactly that descriptor's bytes to exactly
    /// its fd; a disarmed (negative-fd) descriptor skips the write and
    /// `is_armed()` reports it. One test fn: `install_death_path` mutates
    /// process-global state (hook + descriptor). The raw signal half is proven
    /// by T8's `selftest-panic` fixture test + the hw gate.
    #[test]
    fn l2_panic_hook_writes_descriptor_bytes_and_a_negative_fd_is_ignored() {
        let path = std::env::temp_dir().join(format!("afanctl-t4-l2-{}", std::process::id()));
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        install_death_path(SafeRestore::new(f.as_raw_fd(), RESTORE));
        assert!(is_armed(), "a usable descriptor must report armed");
        let payload = catch_unwind(AssertUnwindSafe(|| arm_test_panic())).unwrap_err();
        let msg = payload.downcast_ref::<&str>().unwrap_or(&"");
        assert!(msg.contains("selftest-panic"), "unexpected payload: {msg}");
        assert_eq!(
            fs::read(&path).unwrap(),
            RESTORE,
            "the hook must write the descriptor's whole byte slice, not one byte"
        );

        install_death_path(SafeRestore::new(-1, RESTORE)); // disarm
        assert!(!is_armed(), "a negative fd must report NOT armed");
        let _ = catch_unwind(AssertUnwindSafe(|| arm_test_panic()));
        assert_eq!(
            fs::read(&path).unwrap(),
            RESTORE,
            "unarmed fd must not be written"
        );

        let _ = fs::remove_file(&path);
    }

    /// The descriptor is a pure carrier: it reports back exactly what the
    /// backend put in, so `smc`'s per-generation tables are what decide the
    /// restore — this module holds no value of its own (RULING F27).
    #[test]
    fn descriptor_reports_back_exactly_what_the_backend_supplied() {
        // Deliberately not any real restore token: this module must carry
        // whatever the backend hands it, and know nothing about its meaning.
        static OPAQUE: &[u8] = b"whatever-the-backend-says";
        let d = SafeRestore::new(7, OPAQUE);
        assert_eq!(d.fd(), 7);
        assert_eq!(d.bytes(), OPAQUE);
    }
}
