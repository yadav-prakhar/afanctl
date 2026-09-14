//! Run modes, the poll loop, L1 per-poll verify/re-assert, and the
//! command/state file handling (PRD R3, R4-L1, R7, R8).
//!
//! Poll order is exactly: watchdog ping → read cmd file → validate/apply
//! mode → read sensors → controller step → act via smc (verify) → L1
//! re-assert → write `state.json` → watchdog ping (RULING F21 R2: pings at
//! both ends bound the gap a long poll can create); `run()` adds the sleep
//! between polls and arms L2 before notifying READY.
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
use crate::policy::{
    Controller, Decision, MilliC, AUTO_RETRY_LOG_POLLS, OFF_TARGET_WARN_POLLS, STALL_POLLS,
    STALL_TACH_EPSILON_RPM, VERIFY_TOLERANCE_RPM, WRITE_FAIL_FALLBACK,
};
use crate::smc::{FanMode, Smc};

/// Errors surfaced by the supervisor (per-module thiserror enum, §7).
#[derive(Debug, thiserror::Error)]
pub enum SupError {
    #[error("supervisor: config invalid for hardware: {0}")]
    Config(#[from] crate::config::ConfigError),
}

/// Filesystem locations for the plugin-facing files (PRD R7/R8), both under
/// `AFANCTL_RUNTIME_DIR` (default `/run/afanctl`) as resolved by main/cli.
///
/// RULING F18 (A4, N-F14-1): `config_source` carries the resolved global
/// `--config` path so the startup evidence line can name it (was: absent).
#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub cmd: std::path::PathBuf,
    pub state: std::path::PathBuf,
    pub config_source: std::path::PathBuf,
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
    /// RULING F20 (R2): a fallback whose own AUTO restore failed still owns
    /// the fan; the poll loop re-attempts `set_mode(Auto)` until verified.
    auto_restore_pending: bool,
    /// RULING F20 (R2): total AUTO-restore attempts made (for the "after N
    /// attempts" success note).
    auto_restore_attempts: u32,
    /// Rate-limit countdown for the pending-AUTO ERROR log.
    auto_retry_log_polls: u32,
    /// INVARIANT: only ever set from a *verified* write result (R1).
    last_written: Option<u32>,
    last_actual: Option<u32>,
    /// RULING F20 (R1): tach from the previous poll — the stall window keys
    /// on movement between polls, not on command changes.
    prev_actual: Option<u32>,
    /// Consecutive failed writes/re-asserts → `WRITE_FAIL_FALLBACK`.
    write_failures: u32,
    /// RULING F20 (R1) stall window: consecutive motionless, off-target polls.
    stall_polls: u32,
    /// RULING F20 (R4) / F21 (R3): current off-target excursion length. The
    /// dwell WARN repeats every `OFF_TARGET_WARN_POLLS` polls while the
    /// excursion persists; it resets only on convergence or degradation.
    off_target_polls: u32,
    /// RULING F20 (R3): set by `run()` when `panic_fd()` was absent — the
    /// "no L2 → observe" invariant then also gates `apply_mode`'s re-arm, so
    /// a cmd file cannot defeat the startup guard on either channel.
    l2_absent: bool,
    cfg_max_rpm: u32,
    hw_min: u32,
    hw_max: u32,
    interval: std::time::Duration,
    /// Raw content of the last applied command; freshness gate for cmd.json.
    last_cmd: Option<String>,
    watchdog_pings: u64,
    /// RULING F22: completed polls since this daemon started (0), advanced
    /// exactly once per `step_once`. Distinct from `watchdog_pings`, which
    /// moves twice per poll (start + end ping, F21 R2) and is the L3 audit
    /// trail; `polls` is the uptime source for `status --json`.
    polls: u64,
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
    /// RULING F18 (A1, additive): the monitor-only/degraded latch, visible to
    /// the plugin. Additive field, schema id stays `afanctl.state.v1`.
    monitor_only: bool,
    /// RULING F20 (R2, additive): a fallback whose AUTO restore failed — the
    /// fan is still Manual and the daemon keeps retrying AUTO every poll.
    /// Schema id stays `afanctl.state.v1` (F18 precedent).
    auto_restore_pending: bool,
    watchdog_pings: u64,
    /// RULING F22 (additive): completed polls since daemon start, one per
    /// `step_once`. Never conflate with `watchdog_pings` (two per poll, L3).
    polls: u64,
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
            auto_restore_pending: false,
            auto_restore_attempts: 0,
            auto_retry_log_polls: 0,
            last_written: None,
            last_actual: None,
            prev_actual: None,
            write_failures: 0,
            stall_polls: 0,
            off_target_polls: 0,
            l2_absent: false,
            hw_min: hw.0,
            hw_max: hw.1,
            last_cmd: None,
            watchdog_pings: 0,
            polls: 0,
            paths: RuntimePaths {
                cmd: paths.cmd.clone(),
                state: paths.state.clone(),
                config_source: paths.config_source.clone(),
            },
            recent_errors: Vec::new(),
        })
    }

    /// Exactly one poll iteration: watchdog ping → read cmd file → read
    /// sensors → decide → act+verify → L1 re-assert → write state.json →
    /// watchdog ping. Returns the report (testable).
    ///
    /// RULING F21 (R2): the watchdog is pinged at the *start* and at the end
    /// of the poll, so a long (or failing-echo) poll cannot extend the ping
    /// gap beyond `max(interval, poll_work)` — the settle windows cost up to
    /// ≈2.7 s per failing write, and an end-of-poll-only ping at
    /// `interval_s = 14` pushed the gap past the 15 s unit budget.
    pub fn step_once(&mut self) -> StepReport {
        let mut notes = Vec::new();
        // 0. watchdog ping (L3, RULING F21 R2): upper-bound the gap a long
        // poll can create; the second ping below closes the poll itself.
        self.ping_watchdog();
        // 1. command file: validate + apply mode (R8).
        self.apply_cmd_file(&mut notes);
        // 1.5 RULING F20 (R2): while an AUTO restore is pending, one
        // best-effort attempt per poll — before any control decision.
        self.auto_restore_retry(&mut notes);
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
        // 4. L1 per-poll verify/re-assert (RULING F16 R2/R3).
        let l1_ok = self.l1_poll(applied, &mut notes);
        // INVARIANT: verified reports the failure verdict this poll; a
        // monitor-only degraded supervisor reports false even when idle.
        let verified = !self.monitor_only && l1_ok;
        // 5. state.json publish (R7). RULING F22: this poll is now complete,
        // so advance the poll counter exactly once, before publishing, so the
        // published `polls` (and `uptime_s`) includes it.
        self.polls += 1;
        self.publish_state(&decision, applied, verified);
        // 6. watchdog ping (L3). step_once never sleeps: the loop does.
        self.ping_watchdog();
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
        // RULING F22: uptime counts from this daemon's start, so the
        // completed-poll counter begins at 0 here (never loaded from disk).
        self.polls = 0;
        // RULING F20 (R3): record the L2 prerequisite on the supervisor itself
        // so `apply_mode` upholds the invariant on the cmd-file channel too.
        self.l2_absent = self.smc.panic_fd().is_none();
        if self.mode != RunMode::Observe && self.l2_absent {
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
            let report = self.step_once();
            // RULING F24 (R7): the mode-change line is INFO from `apply_mode`;
            // the report's *remaining* notes (tracking re-asserts, watchdog
            // notes, the `cmd applied` echoes) are per-poll detail — R7's
            // debug level. One line per poll at most, and only when there is
            // something to say, so a healthy poll stays silent.
            if !report.notes.is_empty() {
                tracing::debug!("{}", report.notes.join("; "));
            }
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
        self.stall_polls = 0;
        self.off_target_polls = 0;
        self.auto_restore_pending = false;
        self.auto_restore_attempts = 0;
        self.auto_retry_log_polls = 0;
    }

    /// One startup evidence line (RULING F14 observability): effective mode,
    /// L2 armed, watchdog notify path, config source, hw band — the journal
    /// previously carried only systemd's lines. RULING F18 (A4, N-F14-1):
    /// `RuntimePaths.config_source` now supplies the resolved `--config` path.
    fn startup_evidence_line(&self, l2_armed: bool) {
        // Presence check only — never a side-effecting WATCHDOG ping.
        let watchdog_notify = std::env::var_os("NOTIFY_SOCKET").is_some();
        tracing::info!(
            "{}",
            self.startup_evidence_message(l2_armed, watchdog_notify)
        );
    }

    /// The startup evidence line as a testable string (RULING F14/F18, T9b
    /// finding 8): every field, including `config_source`, must survive.
    fn startup_evidence_message(&self, l2_armed: bool, watchdog_notify: bool) -> String {
        format!(
            "afanctl daemon startup: mode={}, l2_armed={l2_armed}, watchdog_notify={watchdog_notify}, state_file={}, config_source={}, hw_band={}..{} rpm",
            mode_name(&self.mode),
            self.paths.state.display(),
            self.paths.config_source.display(),
            self.hw_min,
            self.hw_max
        )
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
    ///
    /// RULING F24 (R7): a *real* transition logs at INFO from here — `run()`
    /// discards the `StepReport`, so the report note alone never reached the
    /// journal (a curve soak left no evidence control was ever taken). The
    /// unchanged-mode early-return below stays: a persistent cmd file must
    /// not re-log (freshness gate).
    fn apply_mode(&mut self, requested: RunMode, notes: &mut Vec<String>) {
        // RULING F20 (R3): one "no L2 → observe" invariant, both channels —
        // the startup guard degraded the mode, so a cmd file must not re-arm
        // control behind its back.
        if self.l2_absent && requested != RunMode::Observe {
            let msg = "L2 death path unavailable (panic fd was not opened at startup); refusing to enter control from cmd.json (fix: restart with a writable fan1_manual)";
            tracing::error!("{msg}");
            self.note_recent(msg.to_owned());
            notes.push("cmd refused: no L2".into());
            return;
        }
        if self.monitor_only && requested != RunMode::Observe {
            // A deliberate re-command re-arms control (plugin retry story).
            tracing::info!("re-commanded after monitor-only degradation; re-arming control");
            self.monitor_only = false;
            self.write_failures = 0;
            // Owning the fan again supersedes the pending AUTO release
            // (F20 R2; RULING F21 R1: the same supersede holds when a fresh
            // command re-enters control while an observe-release retry was
            // still pending — re-commanding *is* the fresh owner decision).
            self.auto_restore_pending = false;
            self.auto_restore_attempts = 0;
            self.auto_retry_log_polls = 0;
        }
        if requested == self.mode {
            return; // INVARIANT: never re-issue an idempotent mode change.
        }
        let from = self.mode.clone();
        // Leave AUTO behind only when entering a writing control mode.
        if requested == RunMode::Observe && self.mode != RunMode::Observe {
            let fan_state = if self.manual_armed {
                match self.smc.set_mode(FanMode::Auto) {
                    Ok(_) => {
                        self.manual_armed = false;
                        self.last_written = None;
                        self.auto_restore_pending = false;
                        self.auto_restore_attempts = 0;
                        self.auto_retry_log_polls = 0;
                        notes.push("cmd applied: observe (fan1_manual=0)".into());
                        "fan released to AUTO (fan1_manual=0, verified)".to_owned()
                    }
                    Err(e) => {
                        // SAFETY-INVARIANT (RULING F21 R1, the cmd-file
                        // sibling of F20 R2): a failed release still owns the
                        // fan — dropping `manual_armed` here strands it in
                        // Manual, unsupervised (L1 early-returns, nothing
                        // retries). Keep ownership, arm the every-poll
                        // `auto_restore_retry`, and record the truth; the
                        // mode field still becomes Observe (the requested
                        // intent, and it stops speed commands) but state must
                        // say the release is pending until it verifies.
                        self.auto_restore_pending = true;
                        self.auto_restore_attempts = 1;
                        self.auto_retry_log_polls = AUTO_RETRY_LOG_POLLS;
                        self.fail_write(format!("cmd observe: set AUTO error: {e}"), notes);
                        format!("AUTO release FAILED ({e}); fan still Manual, retrying")
                    }
                }
            } else {
                notes.push("cmd applied: observe".into());
                "fan already in AUTO".to_owned()
            };
            tracing::info!("{}", mode_change_message(&from, &requested, &fan_state));
            self.mode = requested;
            return;
        }
        notes.push(format!("cmd applied: {}", mode_name(&requested)));
        // Entering a writing control mode: `set_mode(Manual)` happens on this
        // poll's control write (`act` → `write_controlled`), after this line,
        // so name the arm state on the way rather than claim a verification
        // that has not happened yet. A previous writer mode keeps the fan.
        let fan_state = if self.manual_armed {
            "fan1_manual=1 (already armed)".to_owned()
        } else {
            "arming on the control write this poll".to_owned()
        };
        tracing::info!("{}", mode_change_message(&from, &requested, &fan_state));
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

    /// RULING F16 (R2/R3) — L1 is three independent checks:
    /// - **Mode drift**: `fan1_manual` reads `Auto` while we own the fan ⇒
    ///   re-assert `set_mode(Manual)`; its failures are counted.
    /// - **Tracking**: actual rpm more than `VERIFY_TOLERANCE_RPM` from the
    ///   last written value ⇒ idempotent re-assert of the write, *never
    ///   counted* — a fan still decelerating is not a broken fan.
    /// - **Stall detector** (RULING F20 R1): while armed and off-target, a
    ///   poll counts as progress when the tach moved since the previous poll
    ///   (beyond `STALL_TACH_EPSILON_RPM`) or the fan is within tolerance; a
    ///   command change never resets the window. `STALL_POLLS` motionless,
    ///   off-target polls ⇒ unresponsive actuator ⇒ loud error, AUTO +
    ///   monitor-only. Off-target dwell re-warns every
    ///   `OFF_TARGET_WARN_POLLS` polls while the excursion persists (R4 /
    ///   F21 R3, no degradation).
    ///
    /// Only mode-drift failures, write-syscall errors and echo failures
    /// increment `write_failures` (`WRITE_FAIL_FALLBACK` → fallback).
    fn l1_poll(&mut self, applied: Option<u32>, notes: &mut Vec<String>) -> bool {
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
        // RULING F20 (R1): the stall window keys on tach movement between
        // polls — capture the previous poll's actual before overwriting it.
        let prev_actual = self.prev_actual.replace(fan.rpm);
        if !self.manual_armed {
            self.stall_polls = 0;
            self.off_target_polls = 0;
            return true; // not ours to verify
        }
        if self.auto_restore_pending {
            // RULING F20 (R2): we are trying to release the fan to the
            // firmware — no Manual re-assert, no tracking re-assert, nothing
            // counted; `auto_restore_retry` already attempted AUTO this poll.
            return true;
        }
        let mut failed = false;
        if fan.mode != FanMode::Manual {
            failed = !self.enter_manual(notes);
            if !failed {
                notes.push("L1: manual mode re-asserted".into());
            }
        }
        if let Some(written) = self.last_written {
            let dev = fan.rpm.abs_diff(written);
            // Tracking (R2): physical deviation is an idempotent re-assert;
            // skipped when this poll's act step already wrote the same value.
            if !self.monitor_only && dev > VERIFY_TOLERANCE_RPM && applied.is_none() {
                notes.push(format!(
                    "L1: tracking {dev} rpm from written {written}; re-asserting (not a failure)"
                ));
                match self.smc.write_speed(written) {
                    // INVARIANT: logical state only from verified read-back.
                    Ok(verified) => {
                        self.last_written = Some(verified);
                        notes.push(format!("L1: re-asserted at {verified} rpm"));
                    }
                    Err(e) => {
                        failed = true;
                        self.fail_write(format!("L1 re-assert {written}: {e}"), notes);
                    }
                }
            }
            // Stall detector (RULING F20 R1): keyed on tach movement, not on
            // command changes — a moving command cannot buy a dead actuator a
            // fresh window. Plus off-target dwell visibility (R4 / F21 R3).
            if dev > VERIFY_TOLERANCE_RPM {
                self.off_target_polls = self.off_target_polls.saturating_add(1);
                // RULING F21 (R3): the dwell WARN repeats every
                // `OFF_TARGET_WARN_POLLS` polls while the excursion persists
                // (≈ one line per 30 s at default cadence) — a permanently
                // off-target fan must not fall silent after one WARN.
                // Deliberate trade-off: the stall criterion keys on *any*
                // tach movement (RULING F20 R1 as ruled — both alternative
                // criteria fail worse), so a fan that jitters beyond
                // `STALL_TACH_EPSILON_RPM` while parked off target is
                // reported, repeatedly, not degraded: a per-poll criterion
                // cannot separate "jittering but never converging" from
                // "healthily chasing without converging yet". The excursion
                // counter resets only on convergence (below) or degradation.
                if self.off_target_polls.is_multiple_of(OFF_TARGET_WARN_POLLS) {
                    let msg = format!(
                        "fan rpm off target for {} polls (moving, not stalled)",
                        self.off_target_polls
                    );
                    tracing::warn!("{msg}");
                    self.note_recent(msg.clone());
                    notes.push(msg);
                }
                // INVARIANT: only observed tach movement or convergence
                // resets the stall window; a written-target change resets
                // nothing (the F20 MAJOR-1 hole).
                let moved =
                    prev_actual.is_some_and(|prev| fan.rpm.abs_diff(prev) > STALL_TACH_EPSILON_RPM);
                if moved {
                    self.stall_polls = 0;
                } else {
                    self.stall_polls = self.stall_polls.saturating_add(1);
                    if self.stall_polls >= STALL_POLLS {
                        let msg = format!(
                            "stall detector: fan stuck {dev} rpm off target with a motionless tach for {STALL_POLLS} polls; actuator unresponsive"
                        );
                        tracing::error!("{msg}");
                        self.note_recent(msg.clone());
                        notes.push(msg);
                        self.stall_polls = 0;
                        self.off_target_polls = 0;
                        self.degrade_to_auto(notes);
                        return false;
                    }
                }
            } else {
                // Within tolerance: progress by definition; the excursion
                // closes and a fresh one warns again from a fresh window
                // (the repeating WARN resets only here or on degradation).
                self.stall_polls = 0;
                self.off_target_polls = 0;
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

    /// Fallback: restore AUTO and latch monitor-only so no other fan write
    /// happens until a fresh command (R4). RULING F20 (R2): only an `Ok`
    /// verify commits the release — a failed `set_mode(Auto)` keeps the fan
    /// owned (`manual_armed` stays true) and arms `auto_restore_pending`, so
    /// the poll loop re-attempts AUTO every poll instead of stranding it in
    /// Manual, unsupervised.
    fn degrade_to_auto(&mut self, notes: &mut Vec<String>) {
        self.monitor_only = true;
        self.last_written = None;
        self.write_failures = 0;
        self.stall_polls = 0;
        self.off_target_polls = 0;
        match self.smc.set_mode(FanMode::Auto) {
            Ok(_) => {
                self.manual_armed = false;
                self.auto_restore_pending = false;
                self.auto_restore_attempts = 0;
                let msg = format!(
                    "L1 fallback: {WRITE_FAIL_FALLBACK} failed writes → AUTO restored, degrading to monitor-only"
                );
                tracing::error!("{msg}");
                self.note_recent(msg);
                self.note_recent(
                    "L1 fallback: monitor-only degradation after repeated write failures".into(),
                );
                notes.push("fallback: AUTO + monitor-only".into());
            }
            Err(e) => {
                // SAFETY-INVARIANT: the restore failed, so we still own the
                // fan and must not drop supervision: keep `manual_armed`,
                // arm the every-poll AUTO retry. L2/L3 remain the death nets.
                self.auto_restore_pending = true;
                self.auto_restore_attempts = 1;
                self.auto_retry_log_polls = AUTO_RETRY_LOG_POLLS;
                let msg = format!(
                    "L1 fallback: {WRITE_FAIL_FALLBACK} failed writes → AUTO restore FAILED ({e}); fan still Manual, retrying AUTO every poll, monitor-only"
                );
                tracing::error!("{msg}");
                self.note_recent(msg.clone());
                notes.push(msg);
            }
        }
        notes.push("fallback: monitor-only".into());
    }

    /// RULING F20 (R2): one best-effort `set_mode(Auto)` per poll while an
    /// AUTO restore is pending; the log is rate-limited to one ERROR every
    /// `AUTO_RETRY_LOG_POLLS` polls naming the attempt count. On the first
    /// success (a verified write or the register already reading `Auto`):
    /// clear the flag and note it — never claim success without a verified
    /// read-back, never stop retrying.
    fn auto_restore_retry(&mut self, notes: &mut Vec<String>) {
        if !self.auto_restore_pending {
            return;
        }
        self.auto_restore_attempts = self.auto_restore_attempts.saturating_add(1);
        let register_auto = matches!(
            self.smc.read_fan().ok().map(|f| f.mode),
            Some(FanMode::Auto)
        );
        if !register_auto && self.smc.set_mode(FanMode::Auto).is_err() {
            if self.auto_retry_log_polls == 0 {
                let msg = format!(
                    "AUTO restore still pending: {} failed attempts, fan still in Manual; retrying AUTO every poll",
                    self.auto_restore_attempts
                );
                tracing::error!("{msg}");
                self.note_recent(msg);
                self.auto_retry_log_polls = AUTO_RETRY_LOG_POLLS;
            } else {
                self.auto_retry_log_polls -= 1;
            }
            return;
        }
        self.auto_restore_pending = false;
        self.manual_armed = false;
        self.last_written = None;
        self.stall_polls = 0;
        self.off_target_polls = 0;
        let msg = format!(
            "AUTO restored after {} attempts (fan1_manual=0, verified)",
            self.auto_restore_attempts
        );
        tracing::info!("{msg}");
        self.note_recent(msg.clone());
        notes.push(msg);
    }

    // ---- step 5: state.json (R7) ----

    /// One WATCHDOG=1 notification (L3). RULING F21 (R2): sent at the start
    /// and at the end of every poll.
    fn ping_watchdog(&mut self) {
        self.watchdog_pings += 1;
        let _ = crate::notify::sd_watchdog();
    }

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
            monitor_only: self.monitor_only,
            auto_restore_pending: self.auto_restore_pending,
            watchdog_pings: self.watchdog_pings,
            polls: self.polls,
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

/// RULING F24 (R7): one-line INFO message for a real mode transition.
/// `mode_name(Hold)` already carries the rpm; `fan_state` names the
/// `fan1_manual` outcome on the way (release verified/failed, arm already
/// held, or the pending arm the control write performs).
fn mode_change_message(from: &RunMode, to: &RunMode, fan_state: &str) -> String {
    format!(
        "mode change: {} -> {} ({fan_state})",
        mode_name(from),
        mode_name(to)
    )
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
    use crate::policy::{
        Decision, MilliC, OFF_TARGET_WARN_POLLS, STALL_POLLS, VERIFY_TOLERANCE_RPM,
    };
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

    /// Cold sensor: curve target pins at `min_rpm` (below `low_c`).
    fn cool() -> SensorReading {
        SensorReading {
            label: "Core 0".to_string(),
            milli_c: Some(MilliC(45_000)),
        }
    }

    fn none_label() -> SensorReading {
        SensorReading {
            label: "Core 0".to_string(),
            milli_c: None,
        }
    }

    /// RULING F24 test seam: capture this thread's `tracing` output so a unit
    /// test can assert on emitted lines. Thread-local (`with_default`), so
    /// parallel tests do not cross-talk; DEBUG max level includes INFO.
    fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("log buffer").extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Buf(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .without_time()
            .with_writer({
                let buf = buf.clone();
                move || buf.clone()
            })
            .finish();
        let value = tracing::subscriber::with_default(subscriber, f);
        let text = String::from_utf8_lossy(&buf.0.lock().expect("log buffer")).into_owned();
        (value, text)
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
                config_source: self.0.join("afanctl.toml"),
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
        fn set_tach_lag(&self, rpm_per_poll: u32) {
            self.smc().set_tach_lag(rpm_per_poll);
        }
        fn set_tach_frozen(&self, frozen: bool) {
            self.smc().set_tach_frozen(frozen);
        }
        fn set_latency(&self, latency: Option<std::time::Duration>) {
            self.smc().set_latency(latency);
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
            state["watchdog_pings"], 1,
            "state is published between the two pings (start ping counted, end ping not yet — binding poll order, RULING F21 R2)"
        );
        assert_eq!(state["recent_errors"], serde_json::json!([]));
        assert_eq!(
            state["monitor_only"], false,
            "healthy daemon is not latched"
        );
        assert_eq!(state["polls"], 1, "RULING F22: one completed poll");
        // Second poll: decision moves up the slew; the ping counter lands
        // at 2·2−1 = 3 (two pings per poll, publish between them).
        let before = sup.step_once();
        assert_eq!(before.applied_rpm, Some(2700));
        assert!(before.verified);
        assert_eq!(dir.state()["watchdog_pings"], 3);
        assert_eq!(dir.state()["polls"], 2, "RULING F22: two completed polls");
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

    /// Card check (RULING F24 test 2): a real mode transition emits exactly one
    /// INFO `mode change:` line naming previous → new (hold carries the rpm);
    /// an unchanged command — a re-read or a byte-identical re-issue — emits
    /// nothing, so the freshness gate cannot spam the journal.
    #[test]
    fn mode_change_logs_info_on_a_real_transition_only() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);

        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let (_, logs) = capture_logs(|| sup.step_once());
        assert!(
            logs.contains("mode change: observe -> curve"),
            "a real transition must log at INFO: {logs}"
        );

        // No new command this poll: the freshness gate returns before
        // `apply_mode`, so no mode-change line is emitted.
        let (_, logs) = capture_logs(|| sup.step_once());
        assert!(
            !logs.contains("mode change:"),
            "an unchanged poll must not re-log the mode: {logs}"
        );

        // A byte-identical re-issue is also ignored (freshness gate).
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let (_, logs) = capture_logs(|| sup.step_once());
        assert!(
            !logs.contains("mode change:"),
            "a byte-identical cmd must not re-log: {logs}"
        );

        // A real transition into hold names the rpm.
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let (_, logs) = capture_logs(|| sup.step_once());
        assert!(
            logs.contains("mode change: curve -> hold 3000"),
            "hold must name its rpm: {logs}"
        );
        assert!(logs.contains("(fan1_manual=1"), "already armed: {logs}");

        // A real transition back to observe names the release.
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"observe"}"#);
        let (_, logs) = capture_logs(|| sup.step_once());
        assert!(
            logs.contains("mode change: hold 3000 -> observe"),
            "release must name hold 3000 -> observe: {logs}"
        );
        assert!(logs.contains("released to AUTO"), "release named: {logs}");
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

    /// Card check (RULING F16 test 6): a pure tracking deviation beyond the
    /// tolerance band is an idempotent re-assert — never counted, never a
    /// fallback (the old `l1_reasserts_on_drift` semantics, made explicit).
    #[test]
    fn l1_tracking_deviation_reasserts_without_counting() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once(); // manual + write 3000
                         // Firmware/other process moves the fan away from the written value.
        sm.set_fan_state(2700, FanMode::Manual); // 300 rpm drift > 150
        let rep = sup.step_once();
        assert!(
            rep.notes.iter().any(|n| n.contains("re-asserted at 3000")),
            "tracking deviation must re-assert, notes: {:?}",
            rep.notes
        );
        assert_eq!(
            rep.applied_rpm, None,
            "decision re-write skipped (same target)"
        );
        assert!(
            rep.notes.iter().any(|n| n.contains("not a failure")),
            "the deviation is explicitly labeled tracking, notes: {:?}",
            rep.notes
        );
        // INVARIANT (RULING F16 R2): a decelerating fan is not a failed write.
        assert_eq!(sup.write_failures, 0);
        assert!(!sup.monitor_only);
        let fan = sm.fan();
        assert_eq!(fan.rpm, 3000, "drift re-asserted to last written");
    }

    /// Card check (RULING F16 test 6): deviation within the tolerance band
    /// does NOT re-assert and does not touch the failure counter — the
    /// steady-state tracking band, distinct from the counted failure class.
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
        assert!(rep.verified, "the poll is healthy");
        assert_eq!(sup.write_failures, 0, "nothing counted");
        assert!(!sup.monitor_only, "no fallback from a tracking band");
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
        assert_eq!(
            state["monitor_only"], true,
            "the degradation latch must be visible in state.json (F17b)"
        );
        assert!(
            state["recent_errors"]
                .as_array()
                .expect("recent_errors array")
                .iter()
                .any(|e| e["msg"]
                    .as_str()
                    .is_some_and(|m| m.contains("monitor-only degradation"))),
            "state.json must name the cause: {state}"
        );
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

    // ---- F16 write-verification semantics (RULING F16, ticket tests 1–5) ----

    /// RULING F16 test 1: the hardware event, reproduced on fixtures — tach
    /// 6170, curve target 1200, tach-lag mode at 1500 rpm/poll. The ramp
    /// re-asserts the write each poll (tracking, uncounded) and reaches 1200
    /// with zero `write_failures` and no monitor-only fallback.
    #[test]
    fn f16_hardware_event_converges_without_fallback() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![cool()]);
        sm.set_fan_state(6170, FanMode::Auto); // SMC-owned fast-spinning fan
        sm.set_tach_lag(1500); // real deceleration physics
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        for _ in 0..10 {
            let rep = sup.step_once();
            assert!(rep.verified, "poll must be healthy during the ramp");
        }
        // The fan converged after ~4 polls of bounded-rate slewing.
        let fan = sm.fan();
        assert!(
            fan.rpm.abs_diff(1200) <= VERIFY_TOLERANCE_RPM,
            "tach must converge to the 1200 command, got {}",
            fan.rpm
        );
        assert_eq!(fan.mode, FanMode::Manual, "we still own the fan");
        assert_eq!(sup.write_failures, 0, "tracking is never counted");
        assert!(!sup.monitor_only, "curve mode must survive every ramp");
    }

    /// RULING F16 test 2: takeover — mock in Auto with the tach at 6170 while
    /// a hold command arrives ⇒ manual armed, the write is verified by the
    /// `fan1_output` echo, no fallback.
    #[test]
    fn f16_takeover_from_auto_verifies_by_echo() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(6170, FanMode::Auto);
        sm.set_tach_lag(1500);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        let rep = sup.step_once();
        assert_eq!(rep.applied_rpm, Some(3000), "write verified by echo");
        assert!(sup.manual_armed, "control armed on takeover");
        assert_eq!(
            sm.write_attempts(),
            2,
            "exactly set_mode(Manual) + one verified write"
        );
        assert!(rep.verified);
        assert_eq!(sup.write_failures, 0, "takeover is not a failure");
        assert!(!sup.monitor_only);
    }

    /// RULING F16 test 3: stall — lag mode with the tach frozen ⇒ after
    /// `STALL_POLLS` the actuator is declared unresponsive: loud error,
    /// AUTO restored, monitor-only latched.
    #[test]
    fn f16_frozen_fan_stall_detector_falls_back() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(6170, FanMode::Auto);
        sm.set_tach_lag(1500);
        sm.set_tach_frozen(true); // actuator never responds
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        for _ in 0..STALL_POLLS {
            sup.step_once();
        }
        assert!(sup.monitor_only, "stall must latch monitor-only");
        let fan = sm.fan();
        assert_eq!(fan.mode, FanMode::Auto, "AUTO restored after the stall");
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("stall detector")),
            "loud stall error recorded: {:?}",
            sup.recent_errors
        );
    }

    /// RULING F16 test 4: echo genuinely not taken (fault-injected lag mock)
    /// ⇒ `VerifyFailed` after K=3 per poll ⇒ counted ⇒ fallback at
    /// `WRITE_FAIL_FALLBACK` (the counted class survives the ruling).
    #[test]
    fn f16_echo_not_taken_falls_back_after_k_and_fallback() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_tach_lag(1500);
        sm.set_write_stuck(Some(6170)); // register is write-protected glue
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        assert_eq!(
            sup.step_once().applied_rpm,
            None,
            "an unverified echo never commits"
        );
        for poll in 2..=3 {
            sup.step_once();
            if sup.monitor_only {
                assert_eq!(poll, 3, "fallback exactly at WRITE_FAIL_FALLBACK=3");
            }
        }
        assert!(sup.monitor_only, "echo failures must fall back");
    }

    /// RULING F16 test 5: mode drift — the mock flips `fan1_manual` back to
    /// Auto each poll ⇒ every re-assert is counted ⇒ fallback.
    #[test]
    fn f16_mode_drift_counts_and_falls_back() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_tach_lag(1500);
        sm.set_mode_read_back(Some(FanMode::Auto)); // another agent seizes the fan
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        // Every poll: L1 mode-drift re-assert fails VerifyFailed after K=3
        // attempts, counted once; fallback latches at WRITE_FAIL_FALLBACK=3.
        for poll in 1..=3 {
            sup.step_once();
            if sup.monitor_only {
                assert_eq!(poll, 3, "fallback exactly at WRITE_FAIL_FALLBACK=3");
            }
        }
        assert!(sup.monitor_only);
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("fan1_manual manual write error")),
            "each mode-drift re-assert is recorded: {:?}",
            sup.recent_errors
        );
    }

    // ---- RULING F20 (ticket tests 1–6) ----

    /// 84 °C plateau sensor: mid-band, one step above the hot() plateau.
    fn hot_high() -> SensorReading {
        SensorReading {
            label: "Core 0".to_string(),
            milli_c: Some(MilliC(84_000)),
        }
    }

    /// RULING F20 test 1 (MAJOR 1 regression): sustained mid-band target
    /// motion (alternating temps → the written target changes every poll)
    /// with a frozen tach ⇒ the stall detector fires a few polls past
    /// `STALL_POLLS`: monitor-only latches, the message is recorded. This
    /// case defeated the pre-F20 command-change reset and silently passed.
    #[test]
    fn f20_moving_command_frozen_tach_stall_fires() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(6170, FanMode::Auto);
        sm.set_tach_lag(1500);
        sm.set_tach_frozen(true); // dead actuator: the tach never moves
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        let mut monitor_latched_at = None;
        for poll in 1..=STALL_POLLS + 5 {
            sm.set_sensors(if poll % 2 == 0 {
                vec![hot()]
            } else {
                vec![hot_high()]
            });
            sup.step_once();
            if sup.monitor_only {
                monitor_latched_at = Some(poll);
                break;
            }
        }
        let latched = monitor_latched_at.expect("the stall detector must fire");
        assert!(
            latched <= STALL_POLLS + 3,
            "stall fired at poll {latched}: the moving command must not delay it far past {STALL_POLLS}"
        );
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("stall detector")),
            "the stall message must be recorded: {:?}",
            sup.recent_errors
        );
        assert_eq!(sm.fan().mode, FanMode::Auto, "AUTO restored");
        assert_eq!(dir.state()["monitor_only"], true);
        assert_eq!(
            dir.state()["auto_restore_pending"],
            false,
            "restore verified"
        );
    }

    /// RULING F20 test 2: a healthy fan under the same sustained motion (a
    /// moving tach each poll) ⇒ no stall, no degrade, `write_failures == 0`.
    #[test]
    fn f20_sustained_motion_healthy_fan_never_stalls() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(6170, FanMode::Auto);
        sm.set_tach_lag(1500); // moving fan: the tach tracks every poll
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        for poll in 1..=STALL_POLLS + 5 {
            sm.set_sensors(if poll % 2 == 0 {
                vec![hot()]
            } else {
                vec![hot_high()]
            });
            sup.step_once();
            assert!(!sup.monitor_only, "no stall must latch (poll {poll})");
        }
        assert_eq!(sm.fan().mode, FanMode::Manual, "still owned");
        assert_eq!(sup.write_failures, 0);
    }

    /// RULING F20 test 3 (MAJOR 2 regression): AUTO restore fails
    /// (fault-injected) ⇒ `manual_armed` stays true, `auto_restore_pending`
    /// is set and exposed, the "AUTO restored" success text is absent; then
    /// a later successful attempt clears it with the "after N attempts" note.
    #[test]
    fn f20_failed_auto_restore_is_retried_truthfully() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_write_stuck(Some(HW_MIN)); // 3 failed writes → fallback
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once();
        // Inject the AUTO restore verify failure before the third failure
        // fires the fallback (set_mode(Auto) reads back Manual → VerifyFailed).
        sm.set_mode_read_back(Some(FanMode::Manual));
        for _ in 0..3 {
            sup.step_once();
            if sup.monitor_only {
                break;
            }
        }
        assert!(sup.monitor_only, "fallback latched");
        assert!(sup.manual_armed, "we still own the stranded fan");
        assert!(sup.auto_restore_pending, "the pending flag is armed");
        assert!(
            !sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("AUTO restored")),
            "the success lie must be absent: {:?}",
            sup.recent_errors
        );
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("AUTO restore FAILED")),
            "the truthful failure must be recorded: {:?}",
            sup.recent_errors
        );
        assert_eq!(dir.state()["auto_restore_pending"], true);

        // Two later failing attempts poll after poll (rate-limited log), then
        // the fault clears and the next attempt succeeds.
        for _ in 0..2 {
            sup.step_once();
            assert!(sup.auto_restore_pending, "retry until success");
        }
        sm.set_mode_read_back(None);
        sup.step_once();
        assert!(!sup.auto_restore_pending, "first success clears the flag");
        assert!(!sup.manual_armed, "the fan is released");
        assert_eq!(sm.fan().mode, FanMode::Auto);
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("AUTO restored after 4 attempts")),
            "the 'after N attempts' note must be recorded: {:?}",
            sup.recent_errors
        );
        assert_eq!(dir.state()["auto_restore_pending"], false);
    }

    // ---- RULING F21 ----

    /// RULING F21 (R1, ticket test 1 — the probe-B MAJOR regression): the
    /// fan is owned and a `hold` command is active; the observe release's
    /// own `set_mode(Auto)` fails ⇒ `manual_armed` STAYS true, the pending
    /// flag arms, nothing claims "cmd applied", and state renders the truth
    /// (`monitor_only` false, `auto_restore_pending` true). Then the fault
    /// clears and the every-poll retry releases the fan with the
    /// "after N attempts" note.
    #[test]
    fn f21_failed_observe_release_keeps_ownership_and_retries() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once(); // manual armed + 3000 written (fan is ours)
        assert!(sup.manual_armed);
        sm.set_mode_read_back(Some(FanMode::Manual)); // mode writes never verify
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"observe"}"#);
        let rep = sup.step_once();
        assert_eq!(
            rep.mode,
            RunMode::Observe,
            "the mode field still becomes the requested intent"
        );
        assert!(
            sup.manual_armed,
            "a failed release must NOT drop ownership (the F21 MAJOR)"
        );
        assert!(sup.auto_restore_pending, "the release retry must arm");
        assert!(
            !rep.notes.iter().any(|n| n.contains("cmd applied")),
            "no false applied claim: {:?}",
            rep.notes
        );
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("cmd observe: set AUTO error")),
            "the failure must be recorded: {:?}",
            sup.recent_errors
        );
        assert_eq!(sm.fan().mode, FanMode::Manual, "the fan is still Manual");
        let state = dir.state();
        assert_eq!(state["mode"], "observe");
        assert_eq!(state["monitor_only"], false);
        assert_eq!(
            state["auto_restore_pending"], true,
            "the pending flag is the truth the state must render"
        );

        sm.set_mode_read_back(None); // the fault clears
        sup.step_once();
        assert!(!sup.auto_restore_pending, "the retry cleared the flag");
        assert!(!sup.manual_armed, "the fan is released");
        assert_eq!(sm.fan().mode, FanMode::Auto, "fan1_manual=0");
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("AUTO restored after 3 attempts")),
            "release completes via the retry with the attempts note: {:?}",
            sup.recent_errors
        );
        // attempts: 1 armed at the failure + the arming poll's own retry
        // (2) + the next poll's successful retry (3).
    }

    /// RULING F20 R2 / F21 (R1) killer for T9c finding 7: a variant that
    /// retries the AUTO restore only on alternate polls must FAIL — every
    /// pending poll attempts `set_mode(Auto)`, observable as a write-attempt
    /// delta on every poll (monitor-only latching is false here, so the
    /// retry is the only writer).
    #[test]
    fn f21_auto_restore_retry_genuinely_attempts_every_poll() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once();
        sm.set_mode_read_back(Some(FanMode::Manual));
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"observe"}"#);
        sup.step_once(); // release fails, pending armed
        assert!(sup.auto_restore_pending);
        for poll in 1..=3 {
            let before = sm.write_attempts();
            sup.step_once();
            let after = sm.write_attempts();
            assert!(
                after > before,
                "poll {poll}: the AUTO retry must attempt EVERY poll (delta was 0)"
            );
            assert!(
                sup.auto_restore_pending,
                "still pending while the fault holds"
            );
        }
        sm.set_mode_read_back(None);
        sup.step_once();
        assert!(
            !sup.auto_restore_pending,
            "a clearing fault completes the release"
        );
    }

    /// RULING F21 (R2) observable: state.json is published BETWEEN the two
    /// pings, so after poll N the published count is 2N−1 — the start ping
    /// already counted, the end ping not. An end-only-ping variant publishes
    /// N−1 (0,1,2); a start-only variant N (1,2,3); both fail this pin.
    #[test]
    fn f21_watchdog_pings_at_start_and_end_of_each_poll() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once();
        assert_eq!(dir.state()["watchdog_pings"], 1, "poll 1: 2·1−1");
        sup.step_once();
        assert_eq!(dir.state()["watchdog_pings"], 3, "poll 2: 2·2−1");
        sup.step_once();
        assert_eq!(dir.state()["watchdog_pings"], 5, "poll 3: 2·3−1");
    }

    /// RULING F22 test 1: `polls` advances exactly once per completed poll
    /// while `watchdog_pings` advances twice (start + end ping, F21 R2) —
    /// asserted together so the two counters cannot silently swap meanings.
    #[test]
    fn f22_polls_once_per_poll_and_pings_twice() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        for n in 1..=3u64 {
            sup.step_once();
            assert_eq!(
                dir.state()["polls"],
                n,
                "after {n} step_once calls the published poll count is {n}"
            );
            assert_eq!(
                sup.watchdog_pings,
                2 * n,
                "after {n} completed polls the watchdog has been pinged twice each"
            );
        }
        assert_eq!(sup.watchdog_pings, 2 * sup.polls);
    }

    /// RULING F21 (R2) consequence: even a poll blocked on slow smc work
    /// (injected latency) publishes a count that already includes its start
    /// ping — the gap cannot exceed `max(interval, poll_work)`.
    #[test]
    fn f21_long_poll_still_pings_at_its_start() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"curve"}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_latency(Some(std::time::Duration::from_millis(60)));
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once();
        assert_eq!(
            dir.state()["watchdog_pings"],
            1,
            "a long poll must not defer the start ping past its own work"
        );
    }

    /// RULING F20 test 4 (R3): with no L2 death path, a cmd.json writing mode
    /// is refused — observe stays, zero writes, error recorded.
    #[test]
    fn f20_l2_absent_refuses_cmd_control() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":4000}"#);
        let sm = mock_with(vec![hot()]);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.l2_absent = true; // the startup guard's verdict (panic_fd absent)
        let rep = sup.step_once();
        assert_eq!(rep.mode, RunMode::Observe);
        assert_eq!(sm.write_attempts(), 0, "no control without L2");
        assert!(
            sup.recent_errors
                .iter()
                .any(|e| e.msg.contains("refusing to enter control")),
            "error recorded: {:?}",
            sup.recent_errors
        );
        assert_eq!(dir.state()["mode"], "observe");
    }

    /// RULING F20 test 5a: the stall detector fires at exactly `STALL_POLLS`
    /// polls — not "within" (pinning, T9b finding 6).
    #[test]
    fn f20_stall_fires_exactly_at_stall_polls() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(6170, FanMode::Auto);
        sm.set_tach_lag(1500);
        sm.set_tach_frozen(true);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        for poll in 1..STALL_POLLS {
            sup.step_once();
            assert!(
                !sup.monitor_only && sup.manual_armed,
                "poll {poll} (< {STALL_POLLS}) must NOT degrade"
            );
        }
        sup.step_once(); // the exact STALL_POLLS-th armed, off-target poll
        assert!(sup.monitor_only, "stall fires at exactly STALL_POLLS");
    }

    /// RULING F20 test 6 (R4) / F21 (R3): the off-target dwell WARN fires at
    /// exactly `OFF_TARGET_WARN_POLLS` per excursion and the counter resets
    /// on convergence, so a fresh excursion warns again (the *repeating*
    /// warn within one long excursion is pinned in `tests/pin_constants.rs`).
    #[test]
    fn f20_off_target_dwell_warns_at_window_and_resets_on_convergence() {
        let dir = TempDir::new();
        dir.write_cmd(r#"{"schema":"afanctl.cmd.v1","mode":"hold","rpm":3000}"#);
        let sm = mock_with(vec![hot()]);
        sm.set_fan_state(2700, FanMode::Auto);
        sm.set_tach_lag(1500);
        sm.set_tach_frozen(true);
        let mut sup = supervisor(&sm, RunMode::Observe, &dir);
        sup.step_once(); // arm + write 3000; tach 2700 (300 off)
        let warn_at = |n| format!("off target for {n} polls (moving, not stalled)");
        // Excursion 1: alternate 2700 / 2760 each poll — 60 > 50 epsilon per
        // poll, so the stall window never fills while the dwell accumulates.
        let mut warned_at = None;
        for excursion in 2..=OFF_TARGET_WARN_POLLS {
            sm.set_fan_state(
                if excursion % 2 == 0 { 2760 } else { 2700 },
                FanMode::Manual,
            );
            sup.step_once();
            if sup
                .recent_errors
                .iter()
                .any(|e| e.msg.contains(&warn_at(OFF_TARGET_WARN_POLLS)))
            {
                warned_at = Some(excursion);
                break;
            }
        }
        let warned_at = warned_at.expect("the dwell warn must fire");
        assert_eq!(
            warned_at, OFF_TARGET_WARN_POLLS,
            "warn fires exactly at OFF_TARGET_WARN_POLLS"
        );
        assert!(!sup.monitor_only, "no degradation from a dwell warn");
        assert_eq!(sup.write_failures, 0);
        // Converge: the excursion closes; then a second excursion warns again.
        sm.set_fan_state(3000, FanMode::Manual);
        sup.step_once();
        let warns_before = sup
            .recent_errors
            .iter()
            .filter(|e| e.msg.contains("(moving, not stalled)"))
            .count();
        assert_eq!(
            warns_before, 1,
            "excursion 1 (length == one window) warned exactly once"
        );
        for i in 1..=OFF_TARGET_WARN_POLLS {
            sm.set_fan_state(if i % 2 == 0 { 2760 } else { 2700 }, FanMode::Manual);
            sup.step_once();
        }
        let warns_total = sup
            .recent_errors
            .iter()
            .filter(|e| e.msg.contains("(moving, not stalled)"))
            .count();
        assert!(warns_total >= 2, "the second excursion warns again");
    }

    /// RULING F20 test 5b (R5): the startup evidence line is pinned — every
    /// field, including `config_source`, must survive regressions.
    #[test]
    fn startup_evidence_line_names_config_source() {
        let dir = TempDir::new();
        let sm = mock_with(vec![hot()]);
        let sup = supervisor(&sm, RunMode::Curve, &dir);
        let line = sup.startup_evidence_message(true, false);
        for field in [
            "afanctl daemon startup",
            "mode=curve",
            "l2_armed=true",
            "watchdog_notify=false",
            "state_file=",
            "config_source=",
            "hw_band=1200..7200",
        ] {
            assert!(
                line.contains(field),
                "evidence line must carry `{field}`: {line}"
            );
        }
    }
}
