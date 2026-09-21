//! Cross-module pinning tests for the frozen F20/F16 constants: the echo
//! verify tolerance boundary (a read-back 49 rpm off passes, 51 rpm off
//! fails) and the stall detector firing at exactly `STALL_POLLS` — not
//! "within" (T9b finding 6). RULING F21 (R4) adds the F19/F20/F21 value
//! pins and the two jitter trade-off pins (26-rpm jitter ⇒ motionless ⇒
//! the stall window fills; 60-rpm jitter ⇒ no stall but a *repeating*
//! dwell WARN). No sysfs: mocks only, private tempdirs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use afanctl::config::{Config, ResolvedConfig};
use afanctl::policy::{OFF_TARGET_WARN_POLLS, STALL_POLLS, WRITE_ECHO_TOLERANCE_RPM};
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

// ---- RULING F21 (R4): value pins + the jitter trade-off pins ----

/// Test-side shard of one `MockSmc` shared between the supervisor and the
/// test (same seam as the supervisor harness): the supervisor owns the
/// boxed handle while the test injects tach states between polls.
#[derive(Clone)]
struct SharedMock(Arc<Mutex<MockSmc>>);

impl SharedMock {
    fn with(sensors: Vec<SensorReading>) -> Self {
        let mut m = MockSmc::new(HW_MIN, HW_MAX);
        m.set_sensors(sensors);
        Self(Arc::new(Mutex::new(m)))
    }
    fn smc(&self) -> std::sync::MutexGuard<'_, MockSmc> {
        self.0.lock().expect("mock mutex")
    }
    fn set_fan_state(&self, rpm: u32, mode: FanMode) {
        self.smc().set_fan_state(rpm, mode);
    }
    fn set_tach_lag(&self, rpm_per_poll: u32) {
        self.smc().set_tach_lag(rpm_per_poll);
    }
    fn set_tach_frozen(&self, frozen: bool) {
        self.smc().set_tach_frozen(frozen);
    }
}

impl Smc for SharedMock {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, afanctl::smc::SmcError> {
        self.smc().read_sensors()
    }
    fn read_fan(&self) -> Result<afanctl::smc::FanState, afanctl::smc::SmcError> {
        self.smc().read_fan()
    }
    fn hw_min_rpm(&self) -> u32 {
        self.smc().hw_min_rpm()
    }
    fn hw_max_rpm(&self) -> u32 {
        self.smc().hw_max_rpm()
    }
    fn write_speed(&mut self, rpm: u32) -> Result<u32, afanctl::smc::SmcError> {
        self.smc().write_speed(rpm)
    }
    fn set_mode(&mut self, mode: FanMode) -> Result<afanctl::smc::FanMode, afanctl::smc::SmcError> {
        self.smc().set_mode(mode)
    }
    fn safe_restore(&self) -> Option<afanctl::safety::SafeRestore> {
        self.smc().safe_restore()
    }
    fn probe_safe_restore(
        &mut self,
    ) -> Result<afanctl::safety::SafeRestore, afanctl::smc::SmcError> {
        self.smc().probe_safe_restore()
    }
    fn safety_capabilities(&self) -> afanctl::safety::SafetyCapabilities {
        self.smc().safety_capabilities()
    }
}

fn shared_supervisor_over(
    dir: &std::path::Path,
    state_path: &std::path::Path,
    sm: &SharedMock,
) -> Supervisor {
    Supervisor::new(
        Box::new(sm.clone()),
        &ResolvedConfig {
            config: Config::defaults(),
            low_c: 63,
        },
        RunMode::Observe,
        &RuntimePaths {
            cmd: dir.join("cmd.json"),
            state: state_path.to_path_buf(),
            config_source: dir.join("afanctl.toml"),
        },
    )
    .expect("supervisor over shared mock")
}

/// RULING F21 (R4, T9c finding 5): the frozen F19/F20/F21 constants pinned
/// by value — an editor's slip on any of these fails this pin.
#[test]
fn f21_constants_are_pinned_by_value() {
    use afanctl::config::MAX_INTERVAL_S;
    use afanctl::policy::{
        AUTO_RETRY_LOG_POLLS, ECHO_SETTLE_MS, ECHO_SETTLE_SAMPLES, MODE_SETTLE_MS,
        STALL_TACH_EPSILON_RPM, WRITE_RETRY_MAX,
    };
    assert_eq!(ECHO_SETTLE_MS, 1500);
    assert_eq!(ECHO_SETTLE_SAMPLES, 10);
    assert_eq!(MODE_SETTLE_MS, 1000);
    assert_eq!(WRITE_RETRY_MAX, 1);
    assert_eq!(STALL_TACH_EPSILON_RPM, 50);
    assert_eq!(AUTO_RETRY_LOG_POLLS, 10);
    assert_eq!(OFF_TARGET_WARN_POLLS, 30);
    assert_eq!(MAX_INTERVAL_S, 12);
}

/// RULING F21 (R2): the former cap value 14 is rejected by config
/// validation (the settle windows consumed its watchdog margin); 12 is the
/// legal ceiling.
#[test]
fn f21_max_interval_cap_12_legal_and_14_rejected() {
    let toml = |v: &str| {
        format!(
            "[thresholds]\nhigh = 66\nmax = 86\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n\
             [poll]\ninterval_s = {v}\n"
        )
    };
    let err = Config::from_toml(&toml("14")).expect_err("14 s must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("at most 12 s"),
        "fix names the new bound: {msg}"
    );
    let (parsed, warnings) = Config::from_toml(&toml("12")).expect("12 s is legal");
    assert_eq!(parsed.interval_s, 12);
    assert!(warnings.is_empty());
}

fn jitter_fixture() -> (std::path::PathBuf, SharedMock) {
    let dir = tempdir();
    let sm = SharedMock::with(vec![hot()]);
    sm.set_fan_state(2700, FanMode::Auto); // parked 300 rpm off the hold command
    sm.set_tach_lag(1500);
    sm.set_tach_frozen(true); // the tach only moves where the test moves it
    (dir, sm)
}

/// RULING F21 (R4) trade-off pin, cold side: a 26-rpm delta per poll (≤ the
/// pinned `STALL_TACH_EPSILON_RPM` = 50) while parked off target IS
/// motionless — the stall window fills exactly at `STALL_POLLS` and the fan
/// degrades. Lowering epsilon below 26 breaks this pin.
#[test]
fn f21_sub_epsilon_jitter_while_off_target_is_motionless_and_stalls() {
    let (dir, sm) = jitter_fixture();
    let state_path = dir.join("state.json");
    let mut sup = shared_supervisor_over(&dir, &state_path, &sm);
    std::fs::write(dir.join("cmd.json"), HOLD_CMD).expect("write cmd fixture");
    for poll in 1..STALL_POLLS {
        sm.set_fan_state(if poll % 2 == 0 { 2726 } else { 2700 }, FanMode::Manual);
        sup.step_once();
        assert_eq!(
            read_state(&state_path)["monitor_only"],
            false,
            "poll {poll} (< {STALL_POLLS}): 26 ≤ ε jitter counts as motionless, not yet stalled"
        );
    }
    sup.step_once();
    assert_eq!(
        read_state(&state_path)["monitor_only"],
        true,
        "the stall window fills — 26-rpm jitter is motionless under ε = 50"
    );
    assert_eq!(
        sm.smc().read_fan().expect("fan").mode,
        FanMode::Auto,
        "the stall degrades toward AUTO (fail toward the firmware)"
    );
}

/// RULING F21 (R4) trade-off pin, hot side: a 60-rpm delta per poll
/// (> ε = 50) while parked off target counts as progress — the stall window
/// NEVER fills and the dwell WARN repeats at the `OFF_TARGET_WARN_POLLS`
/// cadence. A 60-rpm case under a once-per-lifetime (or once-per-excursion)
/// warn latch emits < 2 warns → fails; an epsilon raised above 60 stalls →
/// fails. This makes the RULING F20 R1 trade-off intentional and pinned.
#[test]
fn f21_super_epsilon_jitter_never_stalls_but_warns_repeatedly() {
    let (dir, sm) = jitter_fixture();
    let state_path = dir.join("state.json");
    let mut sup = shared_supervisor_over(&dir, &state_path, &sm);
    std::fs::write(dir.join("cmd.json"), HOLD_CMD).expect("write cmd fixture");
    sup.step_once(); // arm + write 3000; tach 2700 → 300 rpm off target
    let mut dwell_warns = 0;
    for poll in 2..=2 * OFF_TARGET_WARN_POLLS + 1 {
        sm.set_fan_state(if poll % 2 == 0 { 2760 } else { 2700 }, FanMode::Manual);
        let rep = sup.step_once();
        dwell_warns += usize::from(rep.notes.iter().any(|n| n.contains("off target for")));
        assert_eq!(
            read_state(&state_path)["monitor_only"],
            false,
            "poll {poll}: 60 > ε jitter must never degrade"
        );
    }
    assert!(
        dwell_warns >= 2,
        "the dwell WARN must repeat every {} polls while the excursion persists (got {dwell_warns})",
        OFF_TARGET_WARN_POLLS
    );
    assert_eq!(
        sm.smc().read_fan().expect("fan").mode,
        FanMode::Manual,
        "reported (repeatedly), never degraded — the deliberate trade-off"
    );
}
