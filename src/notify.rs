//! Hand-rolled sd_notify (PRD R11): READY/WATCHDOG/STATUS notifications via a
//! unix datagram to `$NOTIFY_SOCKET`. No systemd crate.
//!
//! Invariant: when `$NOTIFY_SOCKET` is unset (not run under systemd) every
//! function is a no-op returning `false` — never an error.

/// Notify READY=1. Returns false when not running under systemd.
pub fn sd_ready() -> bool {
    unimplemented!("owned by T4")
}

/// Ping WATCHDOG=1. Returns false when not running under systemd.
pub fn sd_watchdog() -> bool {
    unimplemented!("owned by T4")
}

/// Set STATUS=… for `systemctl status`. Returns false when not running under
/// systemd.
pub fn sd_status(msg: &str) -> bool {
    let _ = msg;
    unimplemented!("owned by T4")
}
