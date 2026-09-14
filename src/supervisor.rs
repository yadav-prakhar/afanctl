//! Run modes, the poll loop, L1 per-poll verify/re-assert, and the
//! command/state file handling (PRD R3, R4-L1, R7, R8).
//!
//! Poll order is exactly: read cmd file → validate/apply mode → read sensors
//! → controller step → act via smc (verify) → L1 re-assert → write
//! `state.json` → watchdog ping → sleep.
//!
//! Invariants: L1 counter reaching `WRITE_FAIL_FALLBACK` → restore AUTO +
//! monitor-only degradation, logged loudly; every write through the
//! write-verify path; logical state never updated from unverified writes.

/// Daemon run modes (PRD R3). Observe is always the startup default and
/// never writes `fan1_manual`/`fan1_output`.
#[derive(Debug, Clone, PartialEq)]
pub enum RunMode {
    Observe,
    Curve,
    Hold(u32),
}

/// Report of exactly one poll iteration — the testable seam for `step_once`.
#[derive(Debug)]
pub struct StepReport {
    pub mode: RunMode,
    pub t_eff: Option<MilliC>,
    pub decision: Decision,
    /// verified write result, None if no write
    pub applied_rpm: Option<u32>,
    /// L1 check outcome this poll
    pub verified: bool,
    /// human-readable events (re-asserts, fallbacks, cmd applied)
    pub notes: Vec<String>,
}

use crate::policy::{Decision, MilliC};

/// Errors surfaced by the supervisor (per-module thiserror enum, §7).
#[derive(Debug, thiserror::Error)]
pub enum SupError {
    #[error("supervisor: {0}")]
    // Detailed variants are designed when the poll loop is implemented;
    // the enum exists from T0 so downstream signatures are frozen.
    Stub(String),
}

/// Filesystem locations for the plugin-facing files (PRD R7/R8).
pub struct RuntimePaths {
    pub cmd: std::path::PathBuf,
    pub state: std::path::PathBuf,
}

/// The supervisor: owns smc, controller, current mode, L1 counters.
pub struct Supervisor {
    // smc, controller, mode, l1 counters, cmd/state file paths (owned by T5)
    _private: (),
}

impl Supervisor {
    /// Wire up a supervisor over a backend, resolved config, start mode, and
    /// runtime file paths.
    pub fn new(
        smc: Box<dyn Smc>,
        cfg: &crate::config::ResolvedConfig,
        start: RunMode,
        paths: &RuntimePaths,
    ) -> Result<Self, SupError> {
        let _ = (smc, cfg, start, paths);
        unimplemented!("owned by T5")
    }

    /// Exactly one poll iteration: read cmd file → read sensors → decide →
    /// act+verify → L1 re-assert → write state.json → watchdog ping. Returns
    /// the report (testable).
    pub fn step_once(&mut self) -> StepReport {
        unimplemented!("owned by T5")
    }

    /// Foreground loop (systemd Type=notify). Installs L2, notifies READY,
    /// runs forever.
    pub fn run(&mut self) -> ! {
        unimplemented!("owned by T5")
    }
}

use crate::smc::Smc;
