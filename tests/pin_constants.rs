//! Cross-module pinning tests for the frozen F20/F16 constants: the echo
//! verify tolerance boundary (a read-back 49 rpm off passes, 51 rpm off
//! fails) and the stall detector firing at exactly `STALL_POLLS` — not
//! "within" (T9b finding 6). No sysfs: mocks only, private tempdirs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use afanctl::config::ResolvedConfig;
use afanctl::policy::{STALL_POLLS, WRITE_ECHO_TOLERANCE_RPM};
use afanctl::smc::{FanMode, MockSmc, SensorReading, Smc};
use afanctl::supervisor::{RunMode, RuntimePaths, Supervisor};

const HW_MIN: u32 = 1200;
const HW_MAX: u32 = 7200;

fn hot() -> SensorReading {
    SensorReading {
        label: "Core 0".to_string(),
        milli_c: Some(crate_milli(80)),
    }
}

fn crate_milli(c: i32) -> afanctl::policy::MilliC {
    afanctl::policy::MilliC(c * 1000)
}

/// The echo tolerance boundary is pinned at `WRITE_ECHO_TOLERANCE_RPM`: a
/// read-back 49 rpm off passes (verified), 51 rpm off fails (`VerifyFailed`).
#[test]
fn echo_tolerance_pin_49_passes_51_fails() {
    let mut smc = MockSmc::new(HW_MIN, HW_MAX);
    smc.set_fan_state(HW_MIN, FanMode::Manual);
    smc.set_write_stuck(Some(3000 - WRITE_ECHO_TOLERANCE_RPM + 1));
    let near = smc.write_speed(3000);
    assert!(
        near.expect("a read-back one under the tolerance boundary must pass") == 3000,
        "write verified by the echo"
    );
    smc.set_write_stuck(Some(3000 - WRITE_ECHO_TOLERANCE_RPM - 1));
    assert!(
        smc.write_speed(3000).is_err(),
        "a read-back one over the tolerance boundary must fail"
    );
}

/// The stall detector fires exactly on the `STALL_POLLS`-th armed,
/// off-target, motionless poll — not "within".
#[test]
fn stall_window_pin_fires_at_exactly_stall_polls() {
    let dir = tempdir();
    let state_path = dir.join("state.json");
    let mut mock = MockSmc::new(HW_MIN, HW_MAX);
    mock.set_sensors(vec![hot()]);
    mock.set_fan_state(6170, FanMode::Auto);
    mock.set_tach_lag(1500);
    mock.set_tach_frozen(true);
    let mut sup = supervisor_over(&dir, &state_path, Box::new(mock));
    std::fs::write(dir.join("cmd.json"), HOLD_CMD).expect("write cmd fixture");
    for poll in 1..STALL_POLLS {
        sup.step_once();
        assert_eq!(
            read_state(&state_path)["monitor_only"],
            false,
            "poll {poll} (< {STALL_POLLS}) must not degrade"
        );
    }
    sup.step_once();
    assert_eq!(
        read_state(&state_path)["monitor_only"],
        true,
        "the stall detector fires exactly at STALL_POLLS"
    );
}

const HOLD_CMD: &str = r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#;

fn supervisor_over(
    dir: &std::path::Path,
    state_path: &std::path::Path,
    smc: Box<dyn Smc>,
) -> Supervisor {
    Supervisor::new(
        smc,
        &ResolvedConfig {
            config: afanctl::config::Config::defaults(),
            low_c: 63,
        },
        RunMode::Observe,
        &RuntimePaths {
            cmd: dir.join("cmd.json"),
            state: state_path.to_path_buf(),
            config_source: dir.join("afanctl.toml"),
        },
    )
    .expect("supervisor over mock")
}

/// Private per-test runtime dir (never the repo fixture, never real /sys).
fn tempdir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "afanctl-pin-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("tempdir");
    dir
}

fn read_state(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("state.json exists"))
        .expect("state parses")
}
