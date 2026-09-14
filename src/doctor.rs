//! Diagnostics + empirical curve comparison (PRD R5 `doctor`, Appendix C).
//!
//! All checks are read-only unless `--roundtrip` (the only write doctor ever
//! does: 2-second manual-mode write test). Checklist lines
//! `PASS|FAIL|WARN — <check> — <detail>`; exit 1 if any FAIL.
//!
//! Checks include: applesmc + coretemp present; fan files present &
//! writable-by-root; sensor plausibility vs Tjmax; `fan1_min/max` readback;
//! config validation; systemd unit health; applesmc layout-change detection
//! (hwmon husk gaining fan attrs → warn); L2 fd armed.

/// Run the doctor suite; returns process exit code (1 if any FAIL).
pub fn run(roundtrip: bool, compare_secs: Option<u64>) -> i32 {
    let _ = (roundtrip, compare_secs);
    unimplemented!("owned by T7")
}
