//! Integration tests (PRD R10, owned by T8): run the real binary with
//! `--sysfs-root` against `tests/fixtures/sysfs/` — `once`, hold → cmd file →
//! daemon applies, `selftest-panic` leaves fixture `fan1_manual == 0`, L1
//! re-assert on induced drift.
//!
//! T0 skeleton: no functional tests yet; this file exists so the gate runs
//! and T8 fills it in place.

#[test]
fn skeleton_integration_placeholder() {
    // T8 owns: once decision output; hold→cmd→daemon applies; selftest-panic
    // exits nonzero with fixture fan1_manual == 0; L1 re-assert on drift.
}

/// Proves the `--features hw` guard itself: without the feature this test
/// does not exist; with the feature it must SKIP cleanly unless
/// `AFANCTL_HWTEST=1` AND real applesmc are present (PRD R10 / §8). This is
/// the gate `cargo test --features hw` must pass without hardware.
#[cfg(feature = "hw")]
#[test]
fn hw_guard_skips_without_hardware() {
    let hwtest_enabled = std::env::var("AFANCTL_HWTEST").is_ok_and(|v| v == "1");
    let applesmc_present = std::path::Path::new("/sys/devices/platform/applesmc.768").exists();
    if !(hwtest_enabled && applesmc_present) {
        eprintln!("skipping: hw tests require AFANCTL_HWTEST=1 and real applesmc (PRD §8)");
    } else {
        // Real-hardware assertions are added only in the supervised gate
        // (§9.3). No test below this point may run without the triple guard.
    }
}
