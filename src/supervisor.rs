//! Run modes, the poll loop, L1 per-poll verify/re-assert, and the
//! command/state file handling (PRD R3, R4-L1, R7, R8).
//!
//! Poll order is exactly: read cmd file → validate/apply mode → read sensors
//! → controller step → act via smc (verify) → L1 re-assert → write
//! `state.json` → watchdog ping; `run()` adds the sleep between polls and
//! arms L2 before notifying READY.
//!
//! Invariants: every write goes through the smc write-verify path; logical
//! state commits only on verified results; `WRITE_FAIL_FALLBACK` consecutive
//! failed writes/re-asserts → restore AUTO + latched monitor-only, logged
//! loudly; the cmd file re-validates each poll (invalid → ignore, keep the
//! previous mode); observe never writes `fan1_manual`/`fan1_output`.

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

use crate::config::ResolvedConfig;
use crate::policy::{Controller, Decision, MilliC, VERIFY_TOLERANCE_RPM, WRITE_FAIL_FALLBACK};
use crate::smc::{FanMode, Smc};

/// Errors surfaced by the supervisor (per-module thiserror enum, §7).
#[derive(Debug, thiserror::Error)]
pub enum SupError {
    #[error("supervisor: config invalid for hardware: {0}")]
    Config(#[from] crate::config::ConfigError),
}

/// Filesystem locations for the plugin-facing files (PRD R7/R8), both under
/// `AFANCTL_RUNTIME_DIR` (default `/run/afanctl`) as resolved by main/cli.
#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub cmd: std::path::PathBuf,
    pub state: std::path::PathBuf,
}

/// The supervisor: owns smc, controller, mode, L1 counters, and the
/// cmd/state file paths. Single poller, no threads (§8).
pub struct Supervisor {
    smc: Box<dyn Smc>,
    controller: Controller,
    mode: RunMode,
    /// Latched degradation after fallback: all writes suppressed, acts like
    /// observe, cleared only by re-commanding curve/hold (R4).
    monitor_only: bool,
    /// We currently own the fan (`fan1_manual` = 1, verified).
    manual_armed: bool,
    /// INVARIANT: only ever set from a *verified* write result (R1).
    last_written: Option<u32>,
    last_actual: Option<u32>,
    /// Consecutive failed writes/re-asserts → `WRITE_FAIL_FALLBACK`.
    write_failures: u32,
    cfg_max_rpm: u32,
    hw_min: u32,
    hw_max: u32,
    interval: std::time::Duration,
    /// Raw content of the last applied command; freshness gate for cmd.json.
    last_cmd: Option<String>,
    watchdog_pings: u64,
    paths: RuntimePaths,
    /// Last 5 errors for `state.json` / `status` (R7 recent_errors).
    recent_errors: Vec<RecentError>,
}

/// One entry for Appendix B state schema `recent_errors` (`{ts,msg}`).
#[derive(Debug, Clone, serde::Serialize)]
struct RecentError {
    ts: String,
    msg: String,
}

/// One poll iteration published as Appendix B `afanctl.state.v1` (R7).
#[derive(serde::Serialize)]
struct StateFile<'a> {
    schema: &'a str,
    ts: String,
    mode: String,
    t_eff_c: Option<f64>,
    target_rpm: Option<u32>,
    last_written_rpm: Option<u32>,
    actual_rpm: Option<u32>,
    verified: bool,
    watchdog_pings: u64,
    recent_errors: &'a [RecentError],
}

/// The Appendex B `afanctl.state.v1` schema id.
const STATE_SCHEMA: &str = "afanctl.state.v1";
/// The Appendix B `afanctl.cmd.v1` schema id (R8).
const CMD_SCHEMA: &str = "afanctl.cmd.v1";
/// state.json ring size (Appendix B: `[ … last 5 … ]`).
const RECENT_ERRORS_MAX: usize = 5;

impl Supervisor {
    /// Wire up a supervisor over a backend, resolved config, start mode, and
    /// runtime file paths. Validates the config band against the hardware.
    pub fn new(
        smc: Box<dyn Smc>,
        cfg: &ResolvedConfig,
        start: RunMode,
        paths: &RuntimePaths,
    ) -> Result<Self, SupError> {
        let hw = (smc.hw_min_rpm(), smc.hw_max_rpm());
        cfg.config.validate(hw)?;
        let start = match start {
            // INVARIANT: a commanded hold is always within the hardware band.
            RunMode::Hold(rpm) => RunMode::Hold(rpm.clamp(hw.0, hw.1)),
            other => other,
        };
        Ok(Self {
            controller: Controller::new(cfg),
            cfg_max_rpm: cfg.config.max_rpm,
            interval: std::time::Duration::from_secs(cfg.config.interval_s),
            smc,
            mode: start,
            monitor_only: false,
            manual_armed: false,
            last_written: None,
            last_actual: None,
            write_failures: 0,
            hw_min: hw.0,
            hw_max: hw.1,
            last_cmd: None,
            watchdog_pings: 0,
            paths: RuntimePaths {
                cmd: paths.cmd.clone(),
                state: paths.state.clone(),
            },
            recent_errors: Vec::new(),
        })
    }

    /// Exactly one poll iteration: read cmd file → read sensors → decide →
    /// act+verify → L1 re-assert → write state.json → watchdog ping. Returns
    /// the report (testable).
    pub fn step_once(&mut self) -> StepReport {
        let mut notes = Vec::new();
        // 1. command file: validate + apply mode (R8).
        self.apply_cmd_file(&mut notes);
        // 2. sensors + controller step: a failed read degrades to an empty
        // set so the controller's loss streak drives fail-toward-AUTO (R4).
        let readings = match self.smc.read_sensors() {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("sensor read failed: {e}");
                tracing::error!("{msg}");
                self.note_recent(msg);
                Vec::new()
            }
        };
        let decision = self.controller_step(&readings);
        // 3. act via smc (verified write path; every decision produces
        // exactly one action — including "write nothing", R2).
        let applied = self.act(&decision, &mut notes);
        // 4. L1 per-poll verify/re-assert (R4).
        let l1_ok = self.l1_poll(&mut notes);
        // INVARIANT: verified reports the failure verdict this poll; a
        // monitor-only degraded supervisor reports false even when idle.
        let verified = !self.monitor_only && l1_ok;
        // 5. state.json publish (R7).
        self.publish_state(&decision, applied, verified);
        // 6. watchdog ping (L3). step_once never sleeps: the loop does.
        self.watchdog_pings += 1;
        let _ = crate::notify::sd_watchdog();
        StepReport {
            mode: self.mode.clone(),
            t_eff: self.controller.t_eff(),
            decision,
            applied_rpm: applied,
            verified,
            notes,
        }
    }

    /// Foreground loop (systemd Type=notify, R4-L3): arms L2, reconciles any
    /// stale manual state left by a killed predecessor, notifies READY, then
    /// step + sleep forever (RULING F14: L2 → reconcile → sd_status/READY →
    /// poll loop).
    pub fn run(&mut self) -> ! {
        if self.mode != RunMode::Observe && self.smc.panic_fd().is_none() {
            tracing::error!(
                "L2 death path unavailable; degrading startup mode to observe (R4: no control without a trustworthy safety net)"
            );
            self.mode = RunMode::Observe;
        }
        let l2_armed = self.smc.panic_fd().is_some();
        if let Some(fd) = self.smc.panic_fd() {
            // INVARIANT: L2 armed before any control write can happen.
            crate::safety::install_death_path(fd);
        }
        // RULING F14: reconcile BEFORE sd_status/READY — the SIGKILL restart
        // is the mechanism that actually restores AUTO (an uncatchable signal
        // cannot run L2), and it must complete before systemd marks us ready.
        self.reconcile_stale_state();
        self.startup_evidence_line(l2_armed);
        let _ = crate::notify::sd_status(&format!("mode={}", mode_name(&self.mode)));
        let _ = crate::notify::sd_ready();
        loop {
            self.step_once();
            std::thread::sleep(self.interval);
        }
    }

    /// Startup reconcile (RULING F14 / PRD R4 fail-toward-AUTO): a SIGKILLed
    /// (or OOM-killed) predecessor cannot run L2, so the process may come up
    /// with `fan1_manual == 1` and no safety net behind it. Read the fan:
    /// `Manual` ⇒ log loudly, restore AUTO on the verified write path, and
    /// record it for `status --json` / the plugin. A failed restore — or a
    /// failed fan read while a writing mode is commanded — degrades to
    /// observe + monitor-only: never command manual mode while AUTO cannot
    /// be restored. `Auto` ⇒ nothing written (idempotent; no write on the
    /// healthy path).
    fn reconcile_stale_state(&mut self) {
        let fan = match self.smc.read_fan() {
            Ok(fan) => fan,
            Err(e) => {
                if self.mode != RunMode::Observe {
                    let msg = format!(
                        "startup reconcile: cannot read fan state: {e} (fix: check applesmc fan files are readable); degrading to observe + monitor-only: AUTO cannot be confirmed"
                    );
                    tracing::error!("{msg}");
                    self.note_recent(msg);
                    self.degrade_startup_to_observe();
                } else {
                    tracing::warn!(
                        "startup reconcile: cannot read fan state: {e} (observe mode writes nothing; continuing without reconcile)"
                    );
                }
                return;
            }
        };
        if fan.mode == FanMode::Auto {
            return; // healthy path: no write (reconcile is idempotent).
        }
        let msg = "startup reconcile: previous process died without restoring AUTO (fan1_manual=1; SIGKILL/OOM-kill cannot run L2); restoring AUTO now";
        tracing::warn!("{msg}");
        match self.smc.set_mode(FanMode::Auto) {
            Ok(_) => {
                self.manual_armed = false;
                self.last_written = None;
                self.note_recent(
                    "startup reconcile: stale manual mode restored to AUTO".to_owned(),
                );
                tracing::info!("startup reconcile: AUTO restored (fan1_manual=0, verified)");
            }
            Err(e) => {
                let msg = format!(
                    "startup reconcile: restoring AUTO failed: {e} (fix: check fan1_manual writability as root); degrading to observe + monitor-only: never command manual while AUTO cannot be restored"
                );
                tracing::error!("{msg}");
                self.note_recent(msg);
                self.degrade_startup_to_observe();
            }
        }
    }

    /// Startup degradation latch (RULING F14 invariant): all writes off, mode
    /// observe — the poll loop's monitor-only branch keeps it that way.
    fn degrade_startup_to_observe(&mut self) {
        self.mode = RunMode::Observe;
        self.monitor_only = true;
        self.manual_armed = false;
        self.last_written = None;
        self.write_failures = 0;
    }

    /// One startup evidence line (RULING F14 observability): effective mode,
    /// L2 armed, watchdog notify path, config source, hw band — the journal
    /// previously carried only systemd's lines. The Appendix-A `Supervisor`
    /// never receives the config path, so the config field carries the state
    /// file provenance it does hold (see DEVIATIONS.md F14).
    fn startup_evidence_line(&self, l2_armed: bool) {
        // Presence check only — never a side-effecting WATCHDOG ping.
        let watchdog_notify = std::env::var_os("NOTIFY_SOCKET").is_some();
        tracing::info!(
            mode = %mode_name(&self.mode),
            l2_armed,
            watchdog_notify,
            state_file = %self.paths.state.display(),
            hw_band = format!("{}..{} rpm", self.hw_min, self.hw_max),
            "afanctl daemon startup"
        );
    }

    // ---- step 1: command file (R8) ----

    /// Read + validate + apply `cmd.json`. Absent file = no command; wrong
    /// schema / unknown mode / bad rpm → loud log + ignore, keep previous
    /// mode (R8); hold rpm clamps to the hardware band (R2, card check).
    fn apply_cmd_file(&mut self, notes: &mut Vec<String>) {
        let raw = match std::fs::read_to_string(&self.paths.cmd) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File removed: no new command; a deterministic file that is
                // recreated with new content re-applies via the last_cmd gate.
                return;
            }
            Err(e) => {
                let msg = format!(
                    "cmd file {}: {e} (fix: check file permissions)",
                    self.paths.cmd.display()
                );
                tracing::warn!("{msg}");
                self.note_recent(msg);
                return;
            }
        };
        // INVARIANT: a command applies only when its content is fresh — a
        // persistent same-mode file must never re-assert (it would un-latch
        // monitor-only degradation every poll).
        if self.last_cmd.as_deref() == Some(raw.as_str()) {
            return;
        }
        let Some(request) = Self::parse_cmd(&raw, notes) else {
            return;
        };
        self.last_cmd = Some(raw);
        self.apply_mode(request, notes);
    }

    /// Validate a `afanctl.cmd.v1` document; `None` = invalid (logged).
    fn parse_cmd(raw: &str, notes: &mut Vec<String>) -> Option<RunMode> {
        let value = match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(v) => v,
            Err(e) => {
                notes.push(format!("cmd.json ignored: parse error: {e}"));
                return None;
            }
        };
        if value.get("schema").and_then(serde_json::Value::as_str) != Some(CMD_SCHEMA) {
            notes.push(format!("cmd.json ignored: schema must be `{CMD_SCHEMA}`"));
            return None;
        }
        let mode = match value.get("mode").and_then(serde_json::Value::as_str) {
            Some(mode) => match mode {
                "observe" => RunMode::Observe,
                "curve" => RunMode::Curve,
                "hold" => {
                    let rpm = match value.get("rpm").and_then(serde_json::Value::as_u64) {
                        Some(rpm) => u32::try_from(rpm).map_err(|_| ()),
                        None => Err(()),
                    };
                    match rpm {
                        Ok(rpm) => RunMode::Hold(rpm),
                        // Out-of-range / missing rpm on hold → keep previous
                        // mode (R8); the CLI already rejects below-fan1_min.
                        Err(_) => {
                            notes.push("cmd.json ignored: `hold` needs an integer rpm".into());
                            return None;
                        }
                    }
                }
                other => {
                    notes.push(format!(
                        "cmd.json ignored: unknown mode `{other}` (fix: observe|curve|hold)"
                    ));
                    return None;
                }
            },
            None => {
                notes.push("cmd.json ignored: missing `mode`".into());
                return None;
            }
        };
        Some(mode)
    }

    /// Apply a validated command: no-op if identical to the current mode;
    /// enter control (manual) or leave it (AUTO) through the verify path.
    fn apply_mode(&mut self, requested: RunMode, notes: &mut Vec<String>) {
        if self.monitor_only && requested != RunMode::Observe {
            // A deliberate re-command re-arms control (plugin retry story).
            tracing::info!("re-commanded after monitor-only degradation; re-arming control");
            self.monitor_only = false;
            self.write_failures = 0;
        }
        if requested == self.mode {
            return; // INVARIANT: never re-issue an idempotent mode change.
        }
        // Leave AUTO behind only when entering a writing control mode.
        if requested == RunMode::Observe && self.mode != RunMode::Observe {
            if self.manual_armed {
                match self.smc.set_mode(FanMode::Auto) {
                    Ok(_) => {
                        self.manual_armed = false;
                        self.last_written = None;
                        notes.push("cmd applied: observe (fan1_manual=0)".into());
                    }
                    Err(e) => {
                        self.manual_armed = false;
                        self.last_written = None;
                        self.fail_write(format!("cmd observe: set AUTO error: {e}"), notes);
                    }
                }
            } else {
                notes.push("cmd applied: observe".into());
            }
            self.mode = requested;
            return;
        }
        notes.push(format!("cmd applied: {}", mode_name(&requested)));
        self.mode = requested;
    }

    // ---- step 2: controller ----

    /// Controller step for the current (possibly degraded) mode.
    fn controller_step(&mut self, readings: &[crate::smc::SensorReading]) -> Decision {
        if self.monitor_only || self.mode == RunMode::Observe {
            self.controller.step_observe(readings)
        } else {
            match &self.mode {
                RunMode::Curve => self.controller.step_curve(readings),
                RunMode::Hold(rpm) => self.controller.step_hold(readings, *rpm),
                RunMode::Observe => self.controller.step_observe(readings),
            }
        }
    }

    // ---- step 3: act via smc (verified writes, R1) ----

    /// Apply a decision. `Observe` (interim sensor loss) writes nothing new;
    /// `ReturnToAuto` restores the firmware; speed targets are clamped to the
    /// hardware band and written through the smc verify path.
    fn act(&mut self, decision: &Decision, notes: &mut Vec<String>) -> Option<u32> {
        if self.monitor_only {
            return None;
        }
        match decision {
            Decision::Observe => None,
            Decision::ReturnToAuto => {
                if self.manual_armed {
                    match self.smc.set_mode(FanMode::Auto) {
                        Ok(_) => {
                            self.manual_armed = false;
                            self.last_written = None;
                            notes.push("fan returned to AUTO (sensor loss)".into());
                        }
                        Err(e) => {
                            self.fail_write(format!("ReturnToAuto: set AUTO error: {e}"), notes);
                        }
                    }
                }
                None
            }
            Decision::SetSpeed(rpm) => self.write_controlled(*rpm, notes),
            Decision::EscalateMax => self.write_controlled(self.cfg_max_rpm, notes),
        }
    }

    /// Write a controlled rpm: ensure manual (verified), clamp, then write.
    /// Skips a redundant re-write of the last verified value — L1 still
    /// re-asserts drift every poll.
    fn write_controlled(&mut self, rpm: u32, notes: &mut Vec<String>) -> Option<u32> {
        if !self.manual_armed && !self.enter_manual(notes) {
            return None;
        }
        let target = rpm.clamp(self.hw_min, self.hw_max);
        if self.last_written == Some(target) {
            return None;
        }
        match self.smc.write_speed(target) {
            // INVARIANT: committed only from a verified result (R1).
            Ok(verified) => {
                self.last_written = Some(verified);
                Some(verified)
            }
            Err(e) => {
                self.fail_write(format!("fan1_output {target}: {e}"), notes);
                None
            }
        }
    }

    /// Put `fan1_manual` into Manual through the verify path; false on failure.
    fn enter_manual(&mut self, notes: &mut Vec<String>) -> bool {
        match self.smc.set_mode(FanMode::Manual) {
            Ok(_) => {
                self.manual_armed = true;
                true
            }
            Err(e) => {
                self.fail_write(format!("fan1_manual manual write error: {e}"), notes);
                false
            }
        }
    }

    // ---- step 4: L1 (R4) ----

    /// Re-read `fan1_manual` + `fan1_input`; re-assert mode/rpm drift. A
    /// failed poll bumps the counter; `WRITE_FAIL_FALLBACK` consecutive
    /// failures → AUTO + monitor-only, logged loudly.
    fn l1_poll(&mut self, notes: &mut Vec<String>) -> bool {
        let fan = match self.smc.read_fan() {
            Ok(fan) => {
                self.last_actual = Some(fan.rpm);
                fan
            }
            Err(e) => {
                let msg = format!("L1 fan read failed: {e}");
                tracing::error!("{msg}");
                self.note_recent(msg);
                self.fail_write("L1 poll defeated by fan read failure".into(), notes);
                return false;
            }
        };
        if !self.manual_armed {
            return true; // not ours to verify
        }
        let mut failed = false;
        if fan.mode != FanMode::Manual {
            failed = !self.enter_manual(notes);
            if !failed {
                notes.push("L1: manual mode re-asserted".into());
            }
        }
        if let Some(written) = self.last_written {
            if fan.rpm.abs_diff(written) > VERIFY_TOLERANCE_RPM {
                notes.push(format!(
                    "L1: driften {drift} rpm from last written {written}; re-asserting",
                    drift = fan.rpm.abs_diff(written)
                ));
                match self.smc.write_speed(written) {
                    // INVARIANT: logical state only from verified read-back.
                    Ok(verified) => {
                        self.last_written = Some(verified);
                        notes.push(format!("L1: drift re-asserted at {verified} rpm"));
                    }
                    Err(e) => {
                        failed = true;
                        self.fail_write(format!("L1 re-assert {written}: {e}"), notes);
                    }
                }
            }
        }
        if failed {
            false
        } else {
            // INVARIANT: the counter only resets on a poll that had a live,
            // healthy written target to verify against (never a no-write poll).
            if self.last_written.is_some() {
                self.write_failures = 0;
            }
            true
        }
    }

    /// One more write/re-assert failure: loud, counted, and escalated to
    /// AUTO + monitor-only at `WRITE_FAIL_FALLBACK` (R4).
    fn fail_write(&mut self, msg: String, notes: &mut Vec<String>) {
        tracing::error!(
            "{msg} (fallback {}/{WRITE_FAIL_FALLBACK})",
            self.write_failures + 1
        );
        self.note_recent(msg);
        notes.push("write/re-assert failed".into());
        self.write_failures += 1;
        if self.write_failures >= WRITE_FAIL_FALLBACK {
            self.degrade_to_auto(notes);
        }
    }

    /// Fallback: restore AUTO (best effort — even this may fail) and latch
    /// monitor-only so never another fan write until a fresh command (R4).
    fn degrade_to_auto(&mut self, notes: &mut Vec<String>) {
        match self.smc.set_mode(FanMode::Auto) {
            Ok(_) => notes.push("fallback: fan1_manual=0 (AUTO)".into()),
            Err(_) => {
                // SAFETY-INVARIANT: if even AUTO fails we stay degraded and
                // keep counting; L2/L3 remain the final nets.
                let msg = "fallback: set AUTO itself failed; monitor-only without AUTO restore";
                tracing::error!("{msg}");
                self.note_recent(msg.to_owned());
            }
        }
        self.monitor_only = true;
        self.manual_armed = false;
        self.last_written = None;
        self.write_failures = 0;
        tracing::error!(
            "L1 fallback: {} failed writes → AUTO restored, degrading to monitor-only",
            WRITE_FAIL_FALLBACK
        );
        self.note_recent(
            "L1 fallback: monitor-only degradation after repeated write failures".into(),
        );
        notes.push("fallback: AUTO + monitor-only".into());
    }

    // ---- step 5: state.json (R7) ----

    /// Append a bounded recent_errors entry.
    fn note_recent(&mut self, msg: String) {
        self.recent_errors.push(RecentError { ts: iso_now(), msg });
        if self.recent_errors.len() > RECENT_ERRORS_MAX {
            self.recent_errors.remove(0);
        }
    }

    /// Rewrite `state.json` atomically every poll (R7); a failure is logged
    /// loudly and never aborts the control loop.
    fn publish_state(&mut self, decision: &Decision, applied: Option<u32>, verified: bool) {
        let set_rpm = |rpm: u32| Some(rpm.clamp(self.hw_min, self.hw_max));
        let target = match decision {
            Decision::SetSpeed(rpm) => set_rpm(*rpm),
            Decision::EscalateMax => Some(self.cfg_max_rpm),
            Decision::Observe | Decision::ReturnToAuto => None,
        };
        let state = StateFile {
            schema: STATE_SCHEMA,
            ts: iso_now(),
            mode: mode_token(&self.mode),
            t_eff_c: self.controller.t_eff().map(|m| m.0 as f64 / 1000.0),
            target_rpm: target,
            last_written_rpm: applied.or(self.last_written),
            actual_rpm: self.last_actual,
            verified,
            watchdog_pings: self.watchdog_pings,
            recent_errors: &self.recent_errors,
        };
        let body = serde_json::to_string(&state);
        let path = self.paths.state.clone();
        match body {
            Ok(body) => {
                if let Err(e) = write_atomic(&path, &body) {
                    // INVARIANT: telemetry failure never aborts control (R7);
                    // it goes into recent_errors instead.
                    let msg = format!("state publish {}: {e}", path.display());
                    tracing::error!("{msg}");
                    self.note_recent(msg);
                }
            }
            Err(e) => {
                let msg = format!("state serialize {}: {e}", path.display());
                tracing::error!("{msg}");
                self.note_recent(msg);
            }
        }
    }
}

/// Atomic tmp+rename write for the state file (R8 pattern; mirrors cli).
fn write_atomic(path: &std::path::Path, body: &str) -> Result<(), std::io::Error> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "state path has no parent")
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Human mode string for STATUS lines (hold carries its target).
fn mode_name(mode: &RunMode) -> String {
    match mode {
        RunMode::Observe => "observe".to_owned(),
        RunMode::Curve => "curve".to_owned(),
        RunMode::Hold(rpm) => format!("hold {rpm}"),
    }
}

/// Schema-token mode name for `state.json` (Appendix B `mode` values).
fn mode_token(mode: &RunMode) -> String {
    match mode {
        RunMode::Observe => "observe".to_owned(),
        RunMode::Curve => "curve".to_owned(),
        RunMode::Hold(_) => "hold".to_owned(),
    }
}

/// UTC timestamp, `YYYY-MM-DDTHH:MM:SSZ` (Appendix B shape, hand-rolled —
/// no extra dependency allowed; R11).
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's civil-from-days inverse for the UTC date part.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // T9-F4: tests are allowlisted (§8)

    use super::*;
    use crate::policy::MilliC;
    use crate::smc::{MockSmc, SensorReading};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    const HW_MIN: u32 = 1200;
    const HW_MAX: u32 = 7200;

    fn cfg() -> ResolvedConfig {
        ResolvedConfig {
            config: crate::config::Config {
                high_c: 66,
                max_c: 86,
                min_rpm: HW_MIN,
                max_rpm: 6200,
                interval_s: 1,
            },
            low_c: 63,
        }
    }

    fn hot() -> SensorReading {
        SensorReading {
            label: "Core 0".to_string(),
            // 80 °C plateau → target f(80) = 1950 rising slew to 4700.
            milli_c: Some(MilliC(80_000)),
        }
    }

    fn none_label() -> SensorReading {
        SensorReading {
            label: "Core 0".to_string(),
            milli_c: None,
        }
    }

    static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "afanctl-t5-{}-{}",
                std::process::id(),
                TMP_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("tempdir");
            Self(dir)
        }
        fn paths(&self) -> RuntimePaths {
            RuntimePaths {
                cmd: self.0.join("cmd.json"),
                state: self.0.join("state.json"),
            }
        }
        fn write_cmd(&self, body: &str) {
            std::fs::write(self.0.join("cmd.json"), body).expect("write cmd.json");
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

    fn supervisor(sm: &SharedMock, start: RunMode, dir: &TempDir) -> Supervisor {
        Supervisor::new(Box::new(sm.clone_shared()), &cfg(), start, &dir.paths())
            .expect("supervisor")
    }

    /// Test-side shard of one `MockSmc` shared between the supervisor and the
    /// test: the supervisor owns a boxed handle while tests inject drift and
    /// faults between polls (the refactor seam the supervisor keeps private).
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
        fn set_sensors(&self, sensors: Vec<SensorReading>) {
            self.smc().set_sensors(sensors);
        }
        fn set_write_stuck(&self, read_back: Option<u32>) {
            self.smc().set_write_stuck(read_back);
        }
        fn set_fan_read_fault(&self, kind: Option<std::io::ErrorKind>) {
            self.smc().set_fan_read_fault(kind);
        }
        fn set_mode_read_back(&self, mode: Option<FanMode>) {
            self.smc().set_mode_read_back(mode);
        }
        fn write_attempts(&self) -> u32 {
            self.smc().write_attempts()
        }
        fn fan(&self) -> crate::smc::FanState {
            self.smc().read_fan().expect("read fan from mock")
        }
        fn clone_shared(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    impl Smc for SharedMock {
        fn read_sensors(&self) -> Result<Vec<crate::smc::SensorReading>, crate::smc::SmcError> {
            self.smc().read_sensors()
        }
        fn read_fan(&self) -> Result<crate::smc::FanState, crate::smc::SmcError> {
            self.smc().read_fan()
        }
        fn hw_min_rpm(&self) -> u32 {
            self.smc().hw_min_rpm()
        }
        fn hw_max_rpm(&self) -> u32 {
            self.smc().hw_max_rpm()
        }
        fn write_speed(&mut self, rpm: u32) -> Result<u32, crate::smc::SmcError> {
            self.smc().write_speed(rpm)
        }
        fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, crate::smc::SmcError> {
            self.smc().set_mode(mode)
        }
        fn panic_fd(&self) -> Option<i32> {
            self.smc().panic_fd()
        }
    }

    fn mock_with(sensors: Vec<SensorReading>) -> SharedMock {
        SharedMock::with(sensors)
    }

    /// Card check: curve start (via cmd file) writes verified, L1 holds,
    /// `state.json` correct after one poll.
    #[test]
    fn curve_start_writes_verified_and_state_file_is_correct() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        let rep = sup.step_once();
        assert_eq!(rep.mode, RunMode::Curve);
        assert_eq!(rep.decision, Decision::SetSpeed(1950));
        assert_eq!(rep.applied_rpm, Some(1950));
        assert!(rep.verified);
        assert!(
            rep.notes.iter().any(|n| n.contains("cmd applied")),
            "notes: {:?}",
            rep.notes
        );
        let state = dir.state();
        assert_eq!(state["schema"], "afanctl.state.v1");
        assert_eq!(state["mode"], "curve");
        assert_eq!(state["t_eff_c"], 80.0);
        assert_eq!(state["target_rpm"], 1950);
        assert_eq!(state["last_written_rpm"], 1950);
        assert_eq!(state["actual_rpm"], 1950);
        assert_eq!(state["verified"], true);
        assert_eq!(
            state["watchdog_pings"], 0,
            "state is published before the ping (binding poll order)"
        );
        assert_eq!(state["recent_errors"], serde_json::json!([]));
        // Second poll: decision moves up the slew; the ping counter lands at 1.
        let before = sup.step_once();
        assert_eq!(before.applied_rpm, Some(2700));
        assert!(before.verified);
        assert_eq!(dir.state()["watchdog_pings"], 1);
    }

    /// Card check: observe startup never writes (firmware owns the fan, R3).
    #[test]
    fn observe_never_writes_smc() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        let rep = sup.step_once();
        assert_eq!(rep.applied_rpm, None);
        assert_eq!(rep.decision, Decision::Observe);
        let fan = sm.fan();
        assert_eq!(fan.mode, FanMode::Auto);
        // No cmd file and no drift path; no state change applied.
        let state = dir.state();
        assert_eq!(state["mode"], "observe");
        assert_eq!(state["last_written_rpm"], serde_json::Value::Null);
    }

    /// Card check: mode transitions via the cmd file (through the verify path).
    #[test]
    fn cmd_file_transitions_apply_and_verify() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let rep = sup.step_once();
        assert_eq!(rep.mode, RunMode::Hold(3000));
        assert_eq!(rep.applied_rpm, Some(3000));
        assert!(sup.manual_armed);
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let rep = sup.step_once();
        assert_eq!(rep.mode, RunMode::Curve);
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"observe"}"#);
        sup.step_once();
        let fan = sm.fan();
        assert_eq!(fan.mode, FanMode::Auto);
        assert!(!sup.manual_armed);
    }

    /// Card check: an invalid cmd is ignored + logged, previous mode kept.
    #[test]
    fn invalid_cmd_is_ignored_and_logged() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Curve, &dir);
        for bad in [
            r#"{"schema":"afanctl.cmd.v1","mode":"turbo"}"#,
            r#"{"schema":"wrong.v1","mode":"curve"}"#,
            r#"{"schema":"afanctl.cmd.v1","mode":"hold"}"#, // missing rpm
            "not json at all",
        ] {
            dir.write_cmd(bad);
            let rep = sup.step_once();
            assert!(
                rep.notes.iter().any(|n| n.contains("ignored")),
                "`{bad}` must be logged ignored, notes: {:?}",
                rep.notes
            );
        }
        assert_eq!(sup.mode, RunMode::Curve, "invalid cmds never change mode");
        assert!(sup.manual_armed, "control state untouched");
    }

    /// Card check: L1 detects drift beyond tolerance and re-asserts (hold —
    /// the curve slew moves every poll, so the fixed hold target isolates L1).
    #[test]
    fn l1_reasserts_on_drift() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once(); // manual + write 3000
                         // Firmware/other process moves the fan away from the written value.
        sm.set_fan_state(2700, FanMode::Manual); // 300 rpm drift > 150
        let rep = sup.step_once();
        assert!(rep.notes.iter().any(|n| n.contains("re-asserted at 3000")));
        assert_eq!(
            rep.applied_rpm, None,
            "decision re-write skipped (same target)"
        );
        let fan = sm.fan();
        assert_eq!(fan.rpm, 3000, "drift re-asserted to last written");
    }

    /// Card check: drift within tolerance does NOT re-assert.
    #[test]
    fn l1_tolerance_band_suppresses_reassert() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once(); // written 3000
        sm.set_fan_state(2900, FanMode::Manual); // 100 rpm === within tolerance
        let rep = sup.step_once();
        assert!(
            !rep.notes.iter().any(|n| n.contains("re-asserted at")),
            "within-tolerance drift must not re-assert, notes: {:?}",
            rep.notes
        );
    }

    /// Card check: three consecutive failed writes → AUTO + monitor-only;
    /// further polls in that latched state never write until re-commanded.
    #[test]
    fn fallback_after_write_failures_degrades_to_monitor_only() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_write_stuck(Some(HW_MIN)); // writes never take
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        assert_eq!(
            sup.step_once().applied_rpm,
            None,
            "stuck write never verifies"
        );
        // One failed write per poll; escalate to fallback on the 3rd poll.
        for poll in 2..=3 {
            sup.step_once();
            if sup.monitor_only {
                assert_eq!(poll, 3, "fallback exactly at WRITE_FAIL_FALLBACK=3");
            }
        }
        assert!(sup.monitor_only, "fallback must latch after 3 failed polls");
        let attempts_after_fallback = sm.write_attempts();
        // Latched: poll loop keeps observing, never writes again.
        let again = sup.step_once();
        assert_eq!(again.applied_rpm, None);
        assert_eq!(
            sm.write_attempts(),
            attempts_after_fallback,
            "monitor-only must create zero writes"
        );
        let state = dir.state();
        assert_eq!(state["mode"], "curve"); // commanded mode kept; degraded
        assert_eq!(state["verified"], false);
        // A fresh command content re-arms control (plugin retry story); the
        // same persisted file never does (freshness gate).
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":2500}"#);
        sm.set_write_stuck(None);
        let rep = sup.step_once();
        assert!(!sup.monitor_only);
        assert!(rep.applied_rpm.is_some(), "re-armed control writes again");
        assert!(rep.verified);
        // Same file re-published verbatim by a second writer: no re-apply.
        let attempts = sm.write_attempts();
        sup.step_once();
        assert_eq!(
            sm.write_attempts(),
            attempts,
            "unchanged cmd content must not re-assert"
        );
    }

    /// Card check: hold clamps to the hardware band at the write path (R2,
    /// the CLI's below-fan1_min rejection happens earlier, R5/Q2).
    #[test]
    fn hold_clamps_to_hw_range() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":300}"#);
        let rep = sup.step_once();
        assert_eq!(
            rep.applied_rpm,
            Some(HW_MIN),
            "below-hw hold clamps up to fan1_min"
        );
        assert_eq!(
            rep.mode,
            RunMode::Hold(300),
            "commanded value kept for status"
        );
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":99999}"#);
        let rep = sup.step_once();
        assert_eq!(
            rep.applied_rpm,
            Some(HW_MAX),
            "above-hw hold clamps down to fan1_max"
        );
    }

    /// EscalateMax writes the configured max through the verify path.
    #[test]
    fn escalate_max_writes_cfg_max_rpm() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        // Overshoot: t_eff >= max-1 for OVERSHOOT_POLLS polls.
        let sizzle = SensorReading {
            label: "Core 0".to_string(),
            milli_c: Some(MilliC(85_500)),
        };
        let sm = mock_with(vec![sizzle]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once(); // apply hold 3000
        sup.step_once();
        let rep = sup.step_once(); // third hot poll → EscalateMax fires
        assert_eq!(rep.decision, Decision::EscalateMax);
        assert_eq!(rep.applied_rpm, Some(6200));
    }

    /// Full sensor loss in curve → ReturnToAuto via the verify path (R4).
    #[test]
    fn sensor_loss_returns_to_auto() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once();
        sm.set_sensors(vec![none_label()]);
        sup.step_once();
        sup.step_once();
        let rep = sup.step_once();
        assert_eq!(rep.decision, Decision::ReturnToAuto);
        let fan = sm.fan();
        assert_eq!(fan.mode, FanMode::Auto);
        assert!(!sup.manual_armed);
    }

    /// `run()`'s guard: no L2 fd verifies into observe-only (startup shape).
    #[test]
    fn supervisor_new_validates_cfg_band_with_hw() {
        let dir = TempDir::new();
        let cfg = ResolvedConfig {
            config: crate::config::Config {
                high_c: 66,
                max_c: 86,
                min_rpm: 9800,
                max_rpm: 9900, // clamped to hw 7200 → empty band
                interval_s: 1,
            },
            low_c: 63,
        };
        let result = Supervisor::new(
            Box::new(MockSmc::new(HW_MIN, HW_MAX)),
            &cfg,
            RunMode::Observe,
            &dir.paths(),
        );
        let err = match result {
            Err(err) => err,
            Ok(_) => panic!("an empty clamped band must refuse start"),
        };
        assert!(err.to_string().contains("no usable band"), "{err}");
    }

    /// Iso-utc renders a plausible Appendix B shape (tests civil_from_days).
    #[test]
    fn iso_now_shape_and_civil_from_days() {
        let ts = iso_now();
        assert_eq!(ts.len(), 20, "{ts}");
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..=4], "-");
        let (y, m, d) = civil_from_days(0); // 1970-01-01
        assert_eq!((y, m, d), (1970, 1, 1));
        let (y, m, d) = civil_from_days(20_710); // 2026-09-14 (per PRD era)
        assert_eq!((y, m, d), (2026, 9, 14));
    }

    // ---- F14 startup reconcile (RULING F14, ticket tests 1a–1d) ----

    /// 1a: mock reports Manual at startup ⇒ exactly one set_mode(Auto),
    /// no other write.
    #[test]
    fn reconcile_manual_restores_auto_with_a_single_verified_write() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(HW_MIN, FanMode::Manual); // SIGKILLed predecessor
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.reconcile_stale_state();
        assert_eq!(sm.fan().mode, FanMode::Auto, "AUTO restored");
        assert_eq!(
            sm.write_attempts(),
            1,
            "exactly one set_mode(Auto), no other write"
        );
        assert!(!sup.monitor_only);
        assert_eq!(sup.mode, RunMode::Observe);
        assert!(!sup.recent_errors.is_empty(), "restored state recorded");
    }

    /// 1b: mock reports Auto ⇒ zero writes (no write on the healthy path).
    #[test]
    fn reconcile_auto_writes_nothing() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Curve, &dir);
        sup.reconcile_stale_state();
        assert_eq!(sm.write_attempts(), 0, "healthy path must not write");
        assert_eq!(sup.mode, RunMode::Curve, "mode untouched");
        assert!(!sup.monitor_only);
    }

    /// 1c: reconcile read fails while a writing mode is commanded ⇒ degrade
    /// to observe + monitor_only.
    #[test]
    fn reconcile_read_failure_degrades_to_observe_monitor_only() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        sm.set_fan_read_fault(Some(std::io::ErrorKind::PermissionDenied));
        let mut sup = supervisor(&sm, RunMode::Curve, &dir);
        sup.reconcile_stale_state();
        assert_eq!(sup.mode, RunMode::Observe);
        assert!(sup.monitor_only);
        assert_eq!(sm.write_attempts(), 0, "no write on an unconfirmable fan");
    }

    /// 1d: reconcile restore fails (fault-injected) ⇒ observe + monitor_only
    /// + the loud error is logged (recent_errors carries it for status).
    #[test]
    fn reconcile_restore_failure_degrades_and_logs_loudly() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(HW_MIN, FanMode::Manual);
        sm.set_mode_read_back(Some(FanMode::Manual)); // AUTO write never verifies
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.reconcile_stale_state();
        assert_eq!(sup.mode, RunMode::Observe);
        assert!(sup.monitor_only);
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("restoring AUTO failed")),
            "loud error recorded: {:?}",
            sup.recent_errors
        );
    }
}
