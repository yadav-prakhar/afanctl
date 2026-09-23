//! RULING F19 integration tests: the settle-window write verification the
//! hardware found (SMC adopts `F0Tg` on a ~1 s tick). Full supervisor runs
//! over `MockSmc` with injected echo latency; no sysfs, no hardware.
//!
//! Test 1 — echo latency 300 ms ⇒ `write_speed` succeeds inside the window
//! and a full curve run from a fast-spinning fan (actual 6688, command 1200)
//! converges with zero write failures and no monitor-only latch (test 4: a
//! stale echo is never counted as a write failure).
//!
//! Test 2 — echo never adopted ⇒ `VerifyFailed`, then the L1 fallback still
//! restores AUTO and latches monitor-only (genuine failure stays detectable).
//!
//! Tests 3 (set_mode latency) and the doctor stale-binary classifier live at
//! the unit level (`src/smc.rs`, `src/doctor.rs` co-located tests).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // §8: tests allowlisted

use afanctl::config::{Config, ResolvedConfig};
use afanctl::policy::MilliC;
use afanctl::smc::{FanMode, MockSmc, SensorReading, Smc};
use afanctl::supervisor::{RunMode, RuntimePaths, Supervisor};

const HW_MIN: u32 = 1200;
const HW_MAX: u32 = 7200;

fn cfg() -> ResolvedConfig {
    ResolvedConfig::from(&Config::defaults())
}

fn cool() -> SensorReading {
    SensorReading {
        label: "Core 0".to_string(),
        milli_c: Some(MilliC::from_c(45)),
    }
}

static TEMPO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "afanctl-f19-{}-{}",
            std::process::id(),
            TEMPO.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("tempdir");
        Self(dir)
    }
    fn paths(&self) -> RuntimePaths {
        RuntimePaths {
            cmd: self.0.join("cmd.json"),
            state: self.0.join("state.json"),
            config_source: self.0.join("afanctl.toml"),
        }
    }
    fn state(&self) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(self.0.join("state.json")).expect("state.json exists"),
        )
        .expect("state parses as json")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Test-side shard of one `MockSmc` shared between the supervisor and the
/// test: the supervisor owns the boxed handle while the test scripts the
/// hardware between polls (same seam as the supervisor's own harness).
#[derive(Clone)]
struct SharedMock {
    inner: std::sync::Arc<std::sync::Mutex<MockSmc>>,
}

impl SharedMock {
    fn with(sensors: Vec<SensorReading>) -> Self {
        let mut m = MockSmc::new(HW_MIN, HW_MAX);
        m.set_sensors(sensors);
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(m)),
        }
    }
    fn smc(&self) -> std::sync::MutexGuard<'_, MockSmc> {
        self.inner.lock().expect("mock mutex")
    }
    fn set_fan_state(&self, rpm: u32, mode: FanMode) {
        self.smc().set_fan_state(rpm, mode);
    }
    fn set_tach_lag(&self, rpm_per_poll: u32) {
        self.smc().set_tach_lag(rpm_per_poll);
    }
    fn set_echo_latency(&self, latency: Option<std::time::Duration>) {
        self.smc().set_echo_latency(latency);
    }
    fn set_write_stuck(&self, read_back: Option<u32>) {
        self.smc().set_write_stuck(read_back);
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
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, afanctl::smc::SmcError> {
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

/// Test 1 (+ test 4): a 300 ms-adopting echo is accepted inside the settle
/// window; a full curve run from the measured fast-spin start (actual 6688,
/// command 1200, ~3000 rpm/s deceleration) converges with ZERO counted write
/// failures and no monitor-only latch.
#[test]
fn echo_latency_succeeds_and_curve_converges_without_failures() {
    let dir = TempDir::new();
    let sm = SharedMock::with(vec![cool()]);
    sm.set_fan_state(6688, FanMode::Manual);
    sm.set_tach_lag(3000);
    sm.set_echo_latency(Some(std::time::Duration::from_millis(300)));
    let mut sup =
        Supervisor::new(Box::new(sm.clone()), &cfg(), RunMode::Curve, &dir.paths()).expect("sup");
    for _ in 0..4 {
        let rep = sup.step_once();
        assert!(rep.verified, "every write verifies in the settle window");
        assert!(
            !rep.notes
                .iter()
                .any(|n| n.contains("write/re-assert failed")),
            "stale echo must never be a write failure: {:?}",
            rep.notes
        );
    }
    let state = dir.state();
    assert_eq!(state["monitor_only"], false, "no monitor-only latch");
    assert_eq!(
        state["recent_errors"],
        serde_json::json!([]),
        "no write failures counted for latency-3 (the regression)"
    );
    let fan = sm.smc().read_fan().expect("fan");
    assert_eq!(fan.rpm, 1200, "command converged within tolerance band");
    assert_eq!(fan.mode, FanMode::Manual);
}

/// Test 2: echo never adopted (write not taken) ⇒ `VerifyFailed`; the L1
/// fallback still restores AUTO and latches monitor-only (genuine failure
/// stays detectable). Three consecutive failed polls hit the fallback.
#[test]
fn never_adopted_echo_fails_then_fallback_restores_auto() {
    let dir = TempDir::new();
    let sm = SharedMock::with(vec![cool()]);
    sm.set_fan_state(6688, FanMode::Auto);
    sm.set_write_stuck(Some(6688)); // the echo never holds the written value
    let mut sup =
        Supervisor::new(Box::new(sm.clone()), &cfg(), RunMode::Curve, &dir.paths()).expect("sup");
    for _ in 0..3 {
        let rep = sup.step_once();
        assert!(
            rep.applied_rpm.is_none(),
            "a never-adopted echo means every write stays unverified"
        );
    }
    let state = dir.state();
    assert_eq!(state["monitor_only"], true, "genuine failure latches");
    let errors: Vec<String> = state["recent_errors"]
        .as_array()
        .expect("errors array")
        .iter()
        .filter_map(|e| e["msg"].as_str())
        .map(str::to_owned)
        .collect();
    assert!(
        errors.iter().any(|m| m.contains("verify failed")),
        "fallback names the verify failure: {:?}",
        errors
    );
    // AUTO restored: fallback settles the mode through the same verify path.
    assert_eq!(sm.smc().read_fan().expect("fan").mode, FanMode::Auto);
    // No further writes once latched: a fourth poll writes nothing.
    let attempts = sm.smc().write_attempts();
    let _ = sup.step_once();
    assert_eq!(sm.smc().write_attempts(), attempts, "latched is write-free");
}
