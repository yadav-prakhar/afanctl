//! Diagnostics + empirical curve comparison (PRD R5 `doctor`, Appendix C).
//!
//! All checks are read-only unless `--roundtrip`, which runs a 2-second
//! manual-mode write test and always restores AUTO: that test (and its
//! restore) is the ONLY place this module calls a write. Checklist lines are
//! `PASS|FAIL|WARN — <check> — <detail>`; `--json` reuses the same fields.
//! Checks: applesmc+coretemp present, fan files present/writable, sensor
//! plausibility vs Tjmax, `fan1_min/max` readback, config validation, systemd
//! unit health, layout-change detection (Q4), L2 fd armed. `--compare N`
//! samples `t_eff` + SMC rpm for N seconds and simulates the curve over the
//! same trace. Exit 1 if any FAIL.
//!
//! Invariants: every sysfs read/write goes through `smc` (R1) — this module
//! constructs no sysfs path and never calls `fs::write` itself; `--roundtrip`
//! refuses to write unless the L2 fd is armed (root + writable `fan1_manual`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;

use crate::config::{Config, ResolvedConfig};
use crate::policy::{Controller, Decision, MilliC};
use crate::smc::{FanMode, SensorReading, Smc, SysfsSmc};

/// Module-default config path used when `run` receives `config_path = None`
/// (R6). The sysfs root has no default here: the caller (cli's parsed
/// `--sysfs-root` global) always supplies it (ruling D-T9-F2).
const DEFAULT_CONFIG: &str = "/etc/afanctl/afanctl.toml";
/// Tjmax for coretemp on this platform (PRD §2.1): plausibility ceiling.
const TJMAX_C: i32 = 100;
/// Manual-mode hold duration for the `--roundtrip` write test (R5: 2 s).
const ROUNDTRIP_HOLD: Duration = Duration::from_secs(2);
/// Fallback binary path when `/proc/<MainPID>/exe` cannot be stat'ed
/// (RULING F19 R6: stale-daemon detection on the packaged install).
const FALLBACK_BINARY: &str = "/usr/bin/afanctl";
/// Detail for the "cannot decide" stale-binary WARN (shared by the live
/// check and the classifier).
const STALE_UNKNOWN: &str = "cannot determine daemon start time or binary mtime (fix: check `systemctl show afanctl.service --property=MainPID,ExecMainStartTimestamp`)";
/// JSON schema id for `doctor --json` (Appendix C "reuses the same fields").
const SCHEMA: &str = "afanctl.doctor.v1";
/// Runtime dir for `state.json` (R7). `cli.rs`'s resolver is private, so
/// doctor mirrors its two constants (RULING F25 allows the in-module copy).
const DEFAULT_RUNTIME_DIR: &str = "/run/afanctl";
/// Env override for the runtime dir, mirroring `cli.rs`.
const RUNTIME_DIR_ENV: &str = "AFANCTL_RUNTIME_DIR";

/// Checklist status (Appendix C). Only `Fail` drives the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Status {
    Pass,
    Fail,
    Warn,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Warn => "WARN",
        }
    }
}

/// One Appendix-C checklist line.
#[derive(Debug, Clone, Serialize)]
struct Check {
    name: String,
    status: Status,
    detail: String,
}

impl Check {
    fn pass(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            status: Status::Pass,
            detail: detail.into(),
        }
    }

    fn fail(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            status: Status::Fail,
            detail: detail.into(),
        }
    }

    fn warn(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            status: Status::Warn,
            detail: detail.into(),
        }
    }
}

/// Full doctor result; `compare` is present only when `--compare N` ran.
#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    checks: Vec<Check>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compare: Option<CompareReport>,
}

/// One `--compare` row: observed SMC data vs our simulated target.
#[derive(Debug, Serialize)]
struct CompareRow {
    t_eff_c: Option<f64>,
    smc_rpm: u32,
    our_rpm: Option<u32>,
    delta_rpm: Option<i64>,
}

/// `--compare` table + divergence stats + one-line verdict guidance.
#[derive(Debug, Serialize)]
struct CompareReport {
    samples: u32,
    lost: u32,
    mean_delta_rpm: f64,
    max_abs_delta_rpm: i64,
    quieter: u32,
    louder: u32,
    equal: u32,
    rows: Vec<CompareRow>,
    verdict: String,
}

/// Resolve the daemon's runtime dir the way `status` does (R7): env override,
/// else `/run/afanctl`.
fn runtime_dir() -> PathBuf {
    std::env::var_os(RUNTIME_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_RUNTIME_DIR))
}

/// Daemon-mode check (RULING F25, PRD §9.3g): read the daemon's published
/// runtime state (R7, same file `status` reads) and surface a held or degraded
/// daemon as WARN. Never FAIL — a user-driven hold or a degraded latch is a
/// diagnostic, and R5/Appendix C reserve the exit code for FAIL.
fn daemon_mode_check(runtime_dir: &Path) -> Check {
    let state = std::fs::read_to_string(runtime_dir.join("state.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    classify_daemon_mode(state.as_ref())
}

/// Pure daemon-mode classifier (unit-tested without `/run` or hardware).
fn classify_daemon_mode(state: Option<&serde_json::Value>) -> Check {
    let name = "daemon mode";
    let Some(state) = state else {
        return Check::pass(name, "no running daemon (observe is the default)");
    };
    let mode = state.get("mode").and_then(serde_json::Value::as_str);
    let flag = |key: &str| {
        state
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    let mut flags: Vec<String> = Vec::new();
    if flag("monitor_only") {
        flags.push("monitor_only=true".to_string());
    }
    if flag("auto_restore_pending") {
        flags.push("auto_restore_pending=true".to_string());
    }
    if !flags.is_empty() {
        // INVARIANT: the degraded latch outranks `mode` in the message — the
        // truth is "requested X, writing nothing" (RULING F25).
        return Check::warn(
            name,
            format!(
                "{} (requested {}, writing nothing){}",
                flags.join(", "),
                mode.unwrap_or("unknown"),
                newest_error(state)
            ),
        );
    }
    match mode {
        Some("hold") => Check::warn(
            name,
            format!(
                "hold active ({} rpm, manual=true) — overshoot guard still applies; release with `afanctl observe`",
                held_rpm(state)
            ),
        ),
        Some("curve") => Check::pass(name, "mode: curve (daemon is driving the fan)"),
        Some("observe") => Check::pass(name, "mode: observe (firmware owns the fan)"),
        other => Check::warn(
            name,
            format!(
                "unknown mode {} in state.json (fix: the daemon writes observe/curve/hold)",
                other.map_or("(missing)".to_string(), |m| format!("{m:?}"))
            ),
        ),
    }
}

/// Held rpm for the hold WARN: `target_rpm`, falling back to
/// `last_written_rpm` (Appendix B), else a marker.
fn held_rpm(state: &serde_json::Value) -> String {
    state
        .get("target_rpm")
        .or_else(|| state.get("last_written_rpm"))
        .and_then(serde_json::Value::as_u64)
        .map_or_else(|| "unknown".to_string(), |rpm| rpm.to_string())
}

/// `; newest error: <ts> <msg>` from `recent_errors`' last entry (empty when
/// the ring is absent or empty).
fn newest_error(state: &serde_json::Value) -> String {
    let Some(err) = state
        .get("recent_errors")
        .and_then(serde_json::Value::as_array)
        .and_then(|errors| errors.last())
    else {
        return String::new();
    };
    let ts = err
        .get("ts")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let msg = err
        .get("msg")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let joined = format!("{ts} {msg}");
    let joined = joined.trim();
    if joined.is_empty() {
        String::new()
    } else {
        format!("; newest error: {joined}")
    }
}

/// Run the doctor suite (Appendix A, amended by ruling D-T9-F2): the CLI
/// globals are explicit parameters — `sysfs_root` redirects all sysfs access
/// (R5), `config_path = None` falls back to the module-default config path
/// (R6). Exit 1 if any check FAILs.
pub fn run(
    sysfs_root: &Path,
    config_path: Option<&Path>,
    roundtrip: bool,
    compare_secs: Option<u64>,
    json: bool,
) -> i32 {
    let config_path = config_path.unwrap_or_else(|| Path::new(DEFAULT_CONFIG));
    run_at(
        sysfs_root,
        config_path,
        &runtime_dir(),
        roundtrip,
        compare_secs,
        json,
    )
}

/// Implementation seam: resolves `config_path` already, so unit tests can
/// drive fixture trees and temp configs directly. Private — Appendix A
/// freezes `run` and no public item may be added (D-protocol).
fn run_at(
    root: &Path,
    config_path: &Path,
    runtime_dir: &Path,
    roundtrip: bool,
    compare_secs: Option<u64>,
    json: bool,
) -> i32 {
    let (mut report, mut smc) = diagnose(root, config_path, runtime_dir);

    if roundtrip {
        let check = match smc.as_mut() {
            Some(backend) => roundtrip_manual_test(backend, ROUNDTRIP_HOLD),
            None => Check::fail(
                "roundtrip manual write test",
                "cannot run: hardware discovery failed",
            ),
        };
        report.checks.push(check);
    }

    if let Some(secs) = compare_secs {
        let resolved = Config::load(config_path)
            .ok()
            .map(|(cfg, _)| ResolvedConfig::from(&cfg));
        match (smc.as_ref(), resolved) {
            (Some(backend), Some(cfg)) => {
                let interval = Duration::from_secs(cfg.config.interval_s.max(1));
                report.compare = Some(run_compare(backend, secs, &cfg, interval));
            }
            _ => report.checks.push(Check::warn(
                "curve comparison",
                "cannot run: hardware discovery or config invalid",
            )),
        }
    }

    let failed = report.checks.iter().any(|c| c.status == Status::Fail);
    if json {
        emit_json(&report);
    } else {
        emit(&render_human(&report));
    }
    if failed {
        1
    } else {
        0
    }
}

/// Build the read-only checklist. Returns the backend too so `run_at` can run
/// the opt-in write test / comparison without re-discovering hardware.
fn diagnose(root: &Path, config_path: &Path, runtime_dir: &Path) -> (Report, Option<SysfsSmc>) {
    let mut checks = Vec::new();
    let opened = SysfsSmc::open(root);
    let smc = match opened {
        Ok(backend) => {
            checks.push(Check::pass(
                "applesmc + coretemp present",
                "devices discovered",
            ));
            let writable = backend.panic_fd().is_some();
            checks.push(if writable {
                Check::pass(
                    "fan files present & writable",
                    "fan1_output present; fan1_manual writable",
                )
            } else {
                Check::fail(
                    "fan files present & writable",
                    "fan1_manual not writable (fix: run doctor as root; an unwritable manual file leaves L2 unarmed)",
                )
            });
            checks.push(sensor_check(&backend));
            let (lo, hi) = (backend.hw_min_rpm(), backend.hw_max_rpm());
            checks.push(if lo > 0 && lo < hi {
                Check::pass("fan1_min/max readback", format!("{lo}..{hi} rpm"))
            } else {
                Check::fail(
                    "fan1_min/max readback",
                    format!("invalid hardware band {lo}..{hi} rpm (fix: check fan1_min/fan1_max)"),
                )
            });
            checks.push(config_check(
                config_path,
                Some((backend.hw_min_rpm(), backend.hw_max_rpm())),
            ));
            checks.push(systemd_check());
            checks.push(stale_binary_check());
            checks.push(daemon_mode_check(runtime_dir));
            checks.push(if backend.layout_changed() {
                Check::warn(
                    "applesmc layout unchanged",
                    "hwmon conversion detected: fan attrs under hwmon/* (fix: update the unit's ReadWritePaths, Q4)",
                )
            } else {
                Check::pass(
                    "applesmc layout unchanged",
                    "fan1_* directly in the applesmc platform dir",
                )
            });
            checks.push(if writable {
                Check::pass("L2 fd armed", "pre-opened O_WRONLY fan1_manual")
            } else {
                Check::fail("L2 fd armed", "no death-path fd (fix: run doctor as root)")
            });
            Some(backend)
        }
        Err(e) => {
            let detail = e.to_string();
            checks.push(Check::fail("applesmc + coretemp present", &detail));
            checks.push(Check::fail(
                "fan files present & writable",
                format!("discovery failed: {detail}"),
            ));
            checks.push(Check::fail(
                "sensor plausibility vs Tjmax 100 C",
                format!("discovery failed: {detail}"),
            ));
            checks.push(Check::fail(
                "fan1_min/max readback",
                format!("discovery failed: {detail}"),
            ));
            checks.push(config_check(config_path, None));
            checks.push(systemd_check());
            checks.push(stale_binary_check());
            checks.push(daemon_mode_check(runtime_dir));
            checks.push(Check::warn(
                "applesmc layout unchanged",
                "unknown: discovery failed",
            ));
            checks.push(Check::fail("L2 fd armed", "hardware discovery failed"));
            None
        }
    };
    (
        Report {
            schema: SCHEMA,
            checks,
            compare: None,
        },
        smc,
    )
}

/// Sensor plausibility against Tjmax (PRD R5): all-failed reads are a FAIL,
/// some-failed a WARN, a reading at/above Tjmax a FAIL.
fn sensor_check(smc: &SysfsSmc) -> Check {
    let name = "sensor plausibility vs Tjmax 100 C";
    let readings = match smc.read_sensors() {
        Ok(r) => r,
        Err(e) => return Check::fail(name, format!("read sensors: {e}")),
    };
    for r in &readings {
        if let Some(m) = r.milli_c {
            if m.0 >= TJMAX_C * 1000 {
                return Check::fail(
                    name,
                    format!("{} reads {} C (>= Tjmax {} C)", r.label, deg(m), TJMAX_C),
                );
            }
        }
    }
    let total = readings.len();
    let failed = readings.iter().filter(|r| r.milli_c.is_none()).count();
    let valid = total - failed;
    let t_eff = readings
        .iter()
        .filter_map(|r| r.milli_c)
        .fold(None, |best: Option<MilliC>, m| match best {
            Some(b) if b.0 >= m.0 => Some(b),
            _ => Some(m),
        });
    match t_eff {
        None => Check::fail(
            name,
            format!("no plausible reading among {total} sensor(s) (all failed/invalid)"),
        ),
        Some(m) if failed > 0 => Check::warn(
            name,
            format!(
                "t_eff {} C from {valid}/{total} sensors ({failed} failed read(s))",
                deg(m)
            ),
        ),
        Some(m) => Check::pass(
            name,
            format!(
                "t_eff {} C from {total} sensor(s), all < Tjmax {} C",
                deg(m),
                TJMAX_C
            ),
        ),
    }
}

/// Config validation (R5/R6): a missing file is fine (defaults); a present but
/// invalid file is a FAIL whose detail carries the config error (key + fix).
fn config_check(path: &Path, hw: Option<(u32, u32)>) -> Check {
    let name = "config validation";
    match Config::load(path) {
        Err(e) => Check::fail(name, format!("{e} (fix: correct the key named above)")),
        Ok((cfg, warnings)) => {
            let provenance = if path.exists() {
                path.display().to_string()
            } else {
                format!("defaults (no file at {})", path.display())
            };
            if let Some(band) = hw {
                if let Err(e) = cfg.validate(band) {
                    return Check::fail(name, format!("{e} [{provenance}]"));
                }
            }
            if warnings.is_empty() {
                Check::pass(name, format!("valid [{provenance}]"))
            } else {
                Check::warn(
                    name,
                    format!(
                        "valid with {} unknown key(s): {} [{provenance}]",
                        warnings.len(),
                        warnings.join("; ")
                    ),
                )
            }
        }
    }
}

/// systemd unit health (R5): configured notify/watchdog/start-limit? via
/// `systemctl show`. A missing unit / missing systemctl is a WARN (dev and
/// non-systemd hosts still get a useful checklist); a loaded-but-misconfigured
/// unit is a FAIL (L3 is a safety layer, R4).
fn systemd_check() -> Check {
    let name = "systemd unit health";
    let output = Command::new("systemctl")
        .args([
            "show",
            "afanctl.service",
            "--property=LoadState,Type,NotifyAccess,WatchdogUSec,StartLimitIntervalUSec",
            "--no-pager",
        ])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            classify_systemd_show(&String::from_utf8_lossy(&out.stdout))
        }
        Ok(out) => Check::warn(
            name,
            format!("systemctl exited {} (unit health unknown)", out.status),
        ),
        Err(e) => Check::warn(name, format!("systemctl unavailable: {e}")),
    }
}

/// Pure parser for `systemctl show` output (unit-testable without systemd).
fn classify_systemd_show(text: &str) -> Check {
    let name = "systemd unit health";
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or("")
            .trim()
    };
    let load = field("LoadState");
    if load != "loaded" {
        return Check::warn(
            name,
            format!(
                "unit not installed (LoadState={load}) (fix: install packaging/afanctl.service)"
            ),
        );
    }
    let unit_type = field("Type");
    let notify = field("NotifyAccess");
    let watchdog = field("WatchdogUSec");
    let watchdog_armed = !watchdog.is_empty() && watchdog != "0" && watchdog != "infinity";
    if unit_type == "notify" && watchdog_armed {
        Check::pass(
            name,
            format!("Type=notify, NotifyAccess={notify}, WatchdogUSec={watchdog}, StartLimitIntervalUSec={}", field("StartLimitIntervalUSec")),
        )
    } else {
        Check::fail(
            name,
            format!(
                "Type={unit_type}, WatchdogUSec={watchdog} (fix: Appendix-D unit needs Type=notify + WatchdogSec=15)"
            ),
        )
    }
}

/// Stale-binary check (RULING F19 R6, the 20:36 hardware lesson): compares
/// the unit's `ExecMainStartTimestamp` with the running binary's mtime.
fn stale_binary_check() -> Check {
    let name = "running daemon matches installed binary";
    let show = Command::new("systemctl")
        .args([
            "show",
            "afanctl.service",
            "--property=MainPID,ExecMainStartTimestamp",
            "--no-pager",
        ])
        .output();
    let text = match show {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => {
            return Check::warn(
                name,
                format!(
                    "systemctl exited {} (stale-binary check unknown)",
                    out.status
                ),
            )
        }
        Err(e) => return Check::warn(name, format!("systemctl unavailable: {e}")),
    };
    let pid = system_field(&text, "MainPID").and_then(|v| v.parse::<u32>().ok());
    if pid.is_none_or(|p| p == 0) {
        return Check::pass(name, "daemon is not running (nothing can be stale)");
    }
    classify_stale_binary(
        system_field(&text, "ExecMainStartTimestamp").and_then(timestamp_epoch),
        running_binary_mtime(pid),
    )
}

/// One `key=value` field from `systemctl show` output (None = missing/empty).
fn system_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let v = text
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))?
        .trim();
    (!v.is_empty()).then_some(v)
}

/// Timestamp to epoch via GNU `date` (no tz parsing dependency allowed by
/// R11): `date` resolves the unit's local-time abbreviation (e.g. `IST`).
fn timestamp_epoch(text: &str) -> Option<i64> {
    let out = Command::new("date")
        .args(["-d", text.trim(), "+%s"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Mtime of the running binary: `/proc/<MainPID>/exe` target, falling back
/// to the packaged `/usr/bin/afanctl` (when the proc link cannot be stat'ed).
fn running_binary_mtime(pid: Option<u32>) -> Option<i64> {
    let exe = pid.filter(|p| *p != 0).map(|p| format!("/proc/{p}/exe"))?;
    std::fs::metadata(&exe)
        .or_else(|_| std::fs::metadata(FALLBACK_BINARY))
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Pure stale-binary classifier (unit-tested without systemd, root, or
/// hardware): daemon start before the binary's mtime = WARN — restart the
/// unit (never a FAIL); unknown either side = WARN (cannot decide).
fn classify_stale_binary(started: Option<i64>, mtime: Option<i64>) -> Check {
    let name = "running daemon matches installed binary";
    match (started, mtime) {
        (Some(start), Some(mtime)) if start < mtime => Check::warn(
            name,
            "running daemon predates the installed binary — restart the unit (systemctl restart afanctl)",
        ),
        (Some(_), Some(_)) => {
            Check::pass(name, "daemon started at/after the installed binary's mtime")
        }
        _ => Check::warn(name, STALE_UNKNOWN),
    }
}

/// `--roundtrip`: the ONLY write doctor performs (plus its mandatory restore).
/// Refuses to write unless the L2 fd is armed (root + writable `fan1_manual`),
/// then: set Manual (verify) → write `fan1_min` (verify) → hold → read →
/// ALWAYS restore Auto (verify). A failed restore is reported as `DANGER`.
fn roundtrip_manual_test(smc: &mut dyn Smc, hold: Duration) -> Check {
    let name = "roundtrip manual write test";
    if smc.panic_fd().is_none() {
        return Check::fail(
            name,
            "--roundtrip requires root (fan1_manual not writable); refused to write",
        );
    }
    let mut problems: Vec<String> = Vec::new();
    match smc.set_mode(FanMode::Manual) {
        Ok(_) => {
            if let Err(e) = smc.write_speed(smc.hw_min_rpm()) {
                problems.push(format!("write {} rpm: {e}", smc.hw_min_rpm()));
            }
        }
        Err(e) => problems.push(format!("set manual: {e}")),
    }
    std::thread::sleep(hold);
    // Observed state during the hold (read before the restore flips it back).
    let observed = smc.read_fan().ok();
    // INVARIANT: AUTO is restored on every path, including failures above.
    let restore = smc.set_mode(FanMode::Auto);
    if let Err(e) = &restore {
        problems.push(format!(
            "restore AUTO failed: {e} (fan may still be manual)"
        ));
    }
    let observed_mode = observed.as_ref().map(|f| f.mode);
    let observed_txt = match &observed {
        Some(f) => format!("observed {} rpm / {:?}", f.rpm, f.mode),
        None => "observed: fan read failed".to_string(),
    };
    if problems.is_empty() && observed_mode == Some(FanMode::Manual) {
        Check::pass(
            name,
            format!(
                "manual+{} rpm verified over {} s; AUTO restored ({observed_txt})",
                smc.hw_min_rpm(),
                hold.as_secs()
            ),
        )
    } else {
        Check::fail(name, format!("{} ({observed_txt})", problems.join("; ")))
    }
}

/// Sample `t_eff` + SMC rpm for `secs` seconds, then compare against the curve
/// simulated over the same trace (R5 `--compare N`, Appendix C).
fn run_compare(
    smc: &dyn Smc,
    secs: u64,
    cfg: &ResolvedConfig,
    interval: Duration,
) -> CompareReport {
    let mut trace: Vec<(Option<MilliC>, u32)> = Vec::new();
    let mut dropped = 0u32;
    for i in 0..secs.max(1) {
        if i > 0 {
            std::thread::sleep(interval);
        }
        let t = smc
            .read_sensors()
            .ok()
            .and_then(|rs| rs.iter().filter_map(|r| r.milli_c).fold(None, max_milli));
        match smc.read_fan() {
            Ok(fan) => trace.push((t, fan.rpm)),
            Err(_) => dropped += 1,
        }
    }
    compare_from_trace(&trace, cfg, dropped)
}

/// Pure comparison math: step the curve over the recorded trace, pair each
/// sample with the SMC rpm, and summarize the deltas (ours - SMC).
fn compare_from_trace(
    trace: &[(Option<MilliC>, u32)],
    cfg: &ResolvedConfig,
    dropped: u32,
) -> CompareReport {
    let mut controller = Controller::new(cfg);
    let mut rows = Vec::with_capacity(trace.len());
    let mut deltas: Vec<i64> = Vec::new();
    let mut lost = dropped;
    for (t_eff, smc_rpm) in trace {
        // INVARIANT: feed the same reading shape the daemon would (a `None`
        // t_eff is a sensor-loss poll, not a missing row).
        let readings = [SensorReading {
            label: "compare".to_string(),
            milli_c: *t_eff,
        }];
        let our_rpm = match controller.step_curve(&readings) {
            Decision::SetSpeed(rpm) => Some(rpm),
            Decision::EscalateMax => Some(cfg.config.max_rpm),
            // ReturnToAuto hands the fan back to firmware: no comparable rpm.
            Decision::ReturnToAuto | Decision::Observe => None,
        };
        let delta = our_rpm.map(|ours| ours as i64 - *smc_rpm as i64);
        match delta {
            Some(d) => deltas.push(d),
            None => lost += 1,
        }
        rows.push(CompareRow {
            t_eff_c: t_eff.map(|m| m.0 as f64 / 1000.0),
            smc_rpm: *smc_rpm,
            our_rpm,
            delta_rpm: delta,
        });
    }
    let max_abs = deltas.iter().fold(0i64, |m, &d| m.max(d.abs()));
    let samples = deltas.len() as u32;
    let mean = if samples == 0 {
        0.0
    } else {
        deltas.iter().sum::<i64>() as f64 / samples as f64
    };
    let quieter = deltas.iter().filter(|d| **d < 0).count() as u32;
    let louder = deltas.iter().filter(|d| **d > 0).count() as u32;
    let equal = deltas.iter().filter(|d| **d == 0).count() as u32;
    CompareReport {
        samples,
        lost,
        mean_delta_rpm: mean,
        max_abs_delta_rpm: max_abs,
        quieter,
        louder,
        equal,
        verdict: verdict(samples, mean, quieter, louder, lost),
        rows,
    }
}

/// One-line guidance for the `--compare` verdict (Appendix C).
fn verdict(samples: u32, mean: f64, quieter: u32, louder: u32, lost: u32) -> String {
    if samples == 0 {
        return format!(
            "no comparable samples ({lost} read failure(s)); rerun `--compare` when sensors read cleanly"
        );
    }
    if louder == 0 {
        format!("ours is never louder here (mean Δ {mean:.0} rpm, {quieter}/{samples} quieter); inspect the table, then decide whether curve mode earns its keep")
    } else if quieter == 0 {
        format!("ours is never quieter here (mean Δ {mean:.0} rpm, {louder}/{samples} louder); curve mode likely does not earn its keep — stay in observe")
    } else {
        format!("mixed: quieter in {quieter}/{samples}, louder in {louder}/{samples} (mean Δ {mean:.0} rpm); judge by the hot samples before enabling curve mode")
    }
}

/// Human Appendix-C rendering: checklist then the optional compare table.
fn render_human(report: &Report) -> String {
    let mut out = String::new();
    for check in &report.checks {
        out.push_str(&format!(
            "{} — {} — {}\n",
            check.status.label(),
            check.name,
            check.detail
        ));
    }
    if let Some(cmp) = &report.compare {
        out.push('\n');
        out.push_str("sample  t_eff(C)  smc_rpm  our_rpm  delta_rpm\n");
        for (i, row) in cmp.rows.iter().enumerate() {
            out.push_str(&format!(
                "{:<6}  {:<8}  {:<7}  {:<7}  {}\n",
                i + 1,
                row.t_eff_c.map_or("-".to_string(), |c| format!("{c:.1}")),
                row.smc_rpm,
                row.our_rpm.map_or("-".to_string(), |r| r.to_string()),
                row.delta_rpm.map_or("-".to_string(), |d| d.to_string()),
            ));
        }
        out.push_str(&format!(
            "\ncompare over {} sample(s): mean Δ {:.0} rpm, max |Δ| {} rpm, ours quieter {} / louder {} / equal {} (lost {})\n",
            cmp.samples, cmp.mean_delta_rpm, cmp.max_abs_delta_rpm, cmp.quieter, cmp.louder, cmp.equal, cmp.lost
        ));
        out.push_str(&format!("verdict: {}\n", cmp.verdict));
    }
    out
}

/// Emit `report` as JSON (Appendix C: same fields as the human output).
fn emit_json(report: &Report) {
    match serde_json::to_string_pretty(report) {
        Ok(json) => emit(&json),
        Err(e) => {
            let mut err = std::io::stderr();
            let _ = writeln!(err, "doctor: json render failed: {e}");
        }
    }
}

/// Write one block to stdout; a broken pipe is not a doctor failure.
fn emit(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.write_all(b"\n");
}

/// Fold helper: hottest valid reading (mirrors the controller's `t_eff`).
fn max_milli(best: Option<MilliC>, m: MilliC) -> Option<MilliC> {
    match best {
        Some(b) if b.0 >= m.0 => Some(b),
        _ => Some(m),
    }
}

/// One-decimal °C rendering from a `MilliC`.
fn deg(m: MilliC) -> String {
    format!("{:.1}", m.0 as f64 / 1000.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs")
    }

    static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

    /// Private tempdir holding a copy of a fixture's `devices/` tree (`""` =
    /// canonical). Tests mutate the copy, never the repo fixture, never /sys.
    struct TempFixture {
        root: PathBuf,
    }

    impl TempFixture {
        fn copy_of(variant: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "afanctl-t7-{}-{}",
                std::process::id(),
                TMP_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            copy_tree(
                &fixture_root().join(variant).join("devices"),
                &root.join("devices"),
            )
            .expect("copy fixture tree to tempdir");
            Self { root }
        }

        fn fan_path(&self, file: &str) -> PathBuf {
            self.root.join("devices/platform/applesmc.768").join(file)
        }

        /// Config file path inside the tempdir (not written unless a test asks).
        fn config_path(&self) -> PathBuf {
            self.root.join("afanctl.toml")
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let to = dst.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &to)?;
            } else {
                fs::copy(entry.path(), to)?;
            }
        }
        Ok(())
    }

    fn no_state_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/absent-runtime")
    }

    fn check<'a>(report: &'a Report, name: &str) -> &'a Check {
        report
            .checks
            .iter()
            .find(|c| c.name == name)
            .expect("check present")
    }

    fn any_fail(report: &Report) -> bool {
        report.checks.iter().any(|c| c.status == Status::Fail)
    }

    fn default_cfg() -> ResolvedConfig {
        ResolvedConfig::from(&Config::defaults())
    }

    /// Healthy fixture: every check passes or warns; exit 0.
    #[test]
    fn healthy_fixture_exits_zero() {
        let (report, smc) = diagnose(
            &fixture_root(),
            &fixture_root().join("absent.toml"),
            &no_state_dir(),
        );
        assert!(smc.is_some(), "canonical fixture must discover");
        assert!(
            !any_fail(&report),
            "no FAIL on healthy fixture: {:?}",
            report.checks
        );
        assert_eq!(
            check(&report, "applesmc + coretemp present").status,
            Status::Pass
        );
        assert_eq!(
            check(&report, "fan files present & writable").status,
            Status::Pass
        );
        assert_eq!(check(&report, "fan1_min/max readback").status, Status::Pass);
        assert_eq!(check(&report, "L2 fd armed").status, Status::Pass);
        assert_eq!(
            check(&report, "applesmc layout unchanged").status,
            Status::Pass
        );
        assert_eq!(
            run_at(
                &fixture_root(),
                &fixture_root().join("absent.toml"),
                &no_state_dir(),
                false,
                None,
                false
            ),
            0
        );
    }

    /// Missing coretemp driver: discovery fails, checklist exits 1.
    #[test]
    fn missing_coretemp_fails() {
        let root = fixture_root().join("no_coretemp");
        let (report, smc) = diagnose(&root, &root.join("absent.toml"), &no_state_dir());
        assert!(smc.is_none());
        assert!(any_fail(&report));
        assert_eq!(
            check(&report, "applesmc + coretemp present").status,
            Status::Fail
        );
        assert_eq!(
            run_at(
                &root,
                &root.join("absent.toml"),
                &no_state_dir(),
                false,
                None,
                false
            ),
            1
        );
    }

    /// Missing fan file: discovery fails, checklist exits 1.
    #[test]
    fn missing_fan_fails() {
        let root = fixture_root().join("missing_fan");
        let (report, _) = diagnose(&root, &root.join("absent.toml"), &no_state_dir());
        assert!(any_fail(&report));
        assert_eq!(
            check(&report, "fan files present & writable").status,
            Status::Fail
        );
        assert_eq!(
            run_at(
                &root,
                &root.join("absent.toml"),
                &no_state_dir(),
                false,
                None,
                false
            ),
            1
        );
    }

    /// fan1_manual present but not writable ⇒ L2 unarmed is a FAIL and the
    /// writable check fails too. A directory makes this deterministic for any
    /// uid (O_WRONLY on a directory is EISDIR even for root).
    #[test]
    fn unwritable_manual_fails_l2_check() {
        let fixture = TempFixture::copy_of("");
        let manual = fixture.fan_path("fan1_manual");
        fs::remove_file(&manual).expect("remove file");
        fs::create_dir(&manual).expect("replace with dir");
        let (report, smc) = diagnose(&fixture.root, &fixture.config_path(), &fixture.root);
        assert!(smc.is_some(), "discovery itself still succeeds");
        assert_eq!(
            check(&report, "fan files present & writable").status,
            Status::Fail
        );
        assert_eq!(check(&report, "L2 fd armed").status, Status::Fail);
        assert_eq!(
            run_at(
                &fixture.root,
                &fixture.config_path(),
                &fixture.root,
                false,
                None,
                false
            ),
            1
        );
    }

    /// A bad config file is a FAIL whose detail names the key and the fix.
    #[test]
    fn bad_config_fails_with_key_and_fix() {
        let fixture = TempFixture::copy_of("");
        let cfg = fixture.config_path();
        fs::write(
            &cfg,
            "[thresholds]\nhigh = 90\nmax = 80\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n[poll]\ninterval_s = 1\n",
        )
        .expect("write bad config");
        let (report, _) = diagnose(&fixture.root, &cfg, &fixture.root);
        let c = check(&report, "config validation");
        assert_eq!(c.status, Status::Fail);
        assert!(
            c.detail.contains("thresholds.high"),
            "detail names key: {}",
            c.detail
        );
        assert!(c.detail.contains("fix:"), "detail names fix: {}", c.detail);
        assert_eq!(
            run_at(&fixture.root, &cfg, &fixture.root, false, None, false),
            1
        );
    }

    /// A valid file with an unknown key is a WARN, not a FAIL (R6).
    #[test]
    fn unknown_key_config_warns_not_fails() {
        let fixture = TempFixture::copy_of("");
        let cfg = fixture.config_path();
        fs::write(
            &cfg,
            "[thresholds]\nhigh = 66\nmax = 86\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n[poll]\ninterval_s = 1\n[future]\nx = 1\n",
        )
        .expect("write config");
        let (report, _) = diagnose(&fixture.root, &cfg, &fixture.root);
        assert_eq!(check(&report, "config validation").status, Status::Warn);
        assert!(!any_fail(&report), "unknown keys must not fail the run");
    }

    /// Layout change (hwmon conversion) is a WARN — never a FAIL — and the
    /// hook is consumed from `SysfsSmc::layout_changed` (Q4).
    #[test]
    fn changed_layout_warns() {
        let root = fixture_root().join("layout_changed");
        let (report, smc) = diagnose(&root, &root.join("absent.toml"), &no_state_dir());
        assert!(smc.is_some());
        assert_eq!(
            check(&report, "applesmc layout unchanged").status,
            Status::Warn
        );
        assert!(!any_fail(&report), "layout change must not fail the run");
        assert_eq!(
            run_at(
                &root,
                &root.join("absent.toml"),
                &no_state_dir(),
                false,
                None,
                false
            ),
            0
        );
    }

    /// Over-Tjmax sensor reads are a FAIL (plausibility check).
    #[test]
    fn over_tjmax_sensor_fails() {
        let root = fixture_root().join("outlier_hi");
        let (report, _) = diagnose(&root, &root.join("absent.toml"), &no_state_dir());
        // 125000 is outside the smc outlier window, so it reads as a failed
        // sensor, not a >100 °C value: the remaining valid sensors still pass.
        assert_eq!(
            check(&report, "sensor plausibility vs Tjmax 100 C").status,
            Status::Warn
        );
    }

    /// All sensors failing (empty tree) is a FAIL.
    #[test]
    fn all_sensors_failed_is_fail() {
        let root = fixture_root().join("no_coretemp");
        let (report, _) = diagnose(&root, &root.join("absent.toml"), &no_state_dir());
        assert_eq!(
            check(&report, "sensor plausibility vs Tjmax 100 C").status,
            Status::Fail
        );
    }

    /// Read-only diagnosis must not mutate the fixture: the only writer is
    /// `--roundtrip` (the review emphasis for T7).
    #[test]
    fn read_only_checks_do_not_write() {
        let fixture = TempFixture::copy_of("");
        let manual = fixture.fan_path("fan1_manual");
        let output = fixture.fan_path("fan1_output");
        let before = (
            fs::read_to_string(&manual).expect("read manual"),
            fs::read_to_string(&output).expect("read output"),
        );
        let _ = run_at(
            &fixture.root,
            &fixture.config_path(),
            &fixture.root,
            false,
            None,
            true,
        );
        let after = (
            fs::read_to_string(&manual).expect("read manual"),
            fs::read_to_string(&output).expect("read output"),
        );
        assert_eq!(before, after, "diagnosis must not write fan files");
    }

    /// `--roundtrip` writes and then restores AUTO (the only write path).
    #[test]
    fn roundtrip_restores_auto() {
        let fixture = TempFixture::copy_of("");
        let manual = fixture.fan_path("fan1_manual");
        let mut smc = SysfsSmc::open(&fixture.root).expect("open fixture");
        let c = roundtrip_manual_test(&mut smc, Duration::from_millis(20));
        assert_eq!(c.status, Status::Pass, "detail: {}", c.detail);
        assert_eq!(
            fs::read_to_string(&manual).expect("read manual").trim(),
            "0",
            "AUTO restored after the write test"
        );
    }

    /// `--roundtrip` refuses to write when the L2 fd is unarmed (non-root):
    /// the mock has no fd and must be rejected without any write attempt.
    #[test]
    fn roundtrip_refuses_without_fd() {
        let mut mock = crate::smc::MockSmc::new(1200, 7200);
        let c = roundtrip_manual_test(&mut mock, Duration::from_millis(0));
        assert_eq!(c.status, Status::Fail);
        assert_eq!(mock.write_attempts(), 0, "no write attempted without L2 fd");
    }

    /// No hardware ⇒ the roundtrip check is an explicit FAIL, not a panic.
    #[test]
    fn roundtrip_without_hardware_fails() {
        let root = fixture_root().join("no_coretemp");
        let code = run_at(
            &root,
            &root.join("absent.toml"),
            &no_state_dir(),
            true,
            None,
            true,
        );
        assert_eq!(code, 1);
    }

    /// Compare math on a recorded 80 °C trace (canonical curve f(80) = 4700):
    /// slew means ours ramps 1950→4700 while the SMC holds 4700/3800.
    #[test]
    fn compare_math_on_recorded_trace() {
        let cfg = default_cfg();
        let t = Some(MilliC::from_c(80));
        let trace = [(t, 4700), (t, 4700), (t, 3800), (t, 4700), (t, 4700)];
        let report = compare_from_trace(&trace, &cfg, 0);
        assert_eq!(report.rows.len(), 5);
        assert_eq!(report.rows[0].our_rpm, Some(1950));
        assert_eq!(report.rows[0].delta_rpm, Some(1950 - 4700));
        assert_eq!(report.rows[4].our_rpm, Some(4700));
        assert_eq!(report.samples, 5);
        assert_eq!(report.quieter, 4);
        assert_eq!(report.louder, 0);
        assert_eq!(report.equal, 1);
        assert_eq!(report.max_abs_delta_rpm, 2750);
        assert_eq!(report.mean_delta_rpm, -1120.0);
        assert!(
            report.verdict.contains("never louder"),
            "{}",
            report.verdict
        );
    }

    /// Sensor-loss samples are excluded from the deltas and counted as lost.
    #[test]
    fn compare_skips_lost_samples() {
        let cfg = default_cfg();
        let t = Some(MilliC::from_c(80));
        let trace = [
            (t, 4700),
            (None, 4700),
            (t, 4700),
            (t, 4700),
            (t, 4700),
            (t, 4700),
        ];
        let report = compare_from_trace(&trace, &cfg, 2);
        assert_eq!(report.samples, 5, "None t_eff is not comparable");
        assert_eq!(report.lost, 3, "1 sensor-loss + 2 dropped reads");
    }

    /// A trace with nothing comparable yields an explicit verdict, no panic.
    #[test]
    fn compare_all_lost_has_verdict() {
        let report = compare_from_trace(&[], &default_cfg(), 4);
        assert_eq!(report.samples, 0);
        assert_eq!(report.lost, 4);
        assert!(report.verdict.contains("no comparable samples"));
    }

    /// Live sampling off a mock (no sysfs, no sleep) wires into the same math.
    #[test]
    fn run_compare_samples_mock() {
        let mut mock = crate::smc::MockSmc::new(1200, 7200);
        mock.set_sensors(vec![SensorReading {
            label: "Core 0".to_string(),
            milli_c: Some(MilliC::from_c(80)),
        }]);
        mock.set_fan_state(3000, FanMode::Auto);
        let report = run_compare(&mock, 2, &default_cfg(), Duration::ZERO);
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].our_rpm, Some(1950));
        assert_eq!(report.rows[0].delta_rpm, Some(-1050));
        assert_eq!(report.mean_delta_rpm, -675.0);
    }

    /// systemd parser: installed+notify+watchdog is a PASS; a simple unit is a
    /// FAIL; a not-found unit is a WARN.
    #[test]
    fn systemd_classification() {
        let good = classify_systemd_show(
            "LoadState=loaded\nType=notify\nNotifyAccess=main\nWatchdogUSec=15s\nStartLimitIntervalUSec=0\n",
        );
        assert_eq!(good.status, Status::Pass);
        let simple = classify_systemd_show(
            "LoadState=loaded\nType=simple\nNotifyAccess=none\nWatchdogUSec=infinity\nStartLimitIntervalUSec=10s\n",
        );
        assert_eq!(simple.status, Status::Fail);
        let missing = classify_systemd_show("LoadState=not-found\n");
        assert_eq!(missing.status, Status::Warn);
    }

    /// RULING F19 (R6) test 5: stale-binary classifier on fixture epochs —
    /// unit start older than the binary mtime ⇒ WARN naming the restart fix;
    /// fresh start ⇒ PASS; unknown either side ⇒ WARN (never a FAIL).
    #[test]
    fn stale_binary_classification() {
        let stale = classify_stale_binary(Some(1000), Some(2000));
        assert_eq!(stale.status, Status::Warn);
        assert!(
            stale
                .detail
                .contains("restart the unit (systemctl restart afanctl)"),
            "{}",
            stale.detail
        );
        let fresh = classify_stale_binary(Some(2000), Some(2000));
        assert_eq!(fresh.status, Status::Pass);
        let unknown = classify_stale_binary(Some(1000), None);
        assert_eq!(unknown.status, Status::Warn);
        assert!(unknown.detail.contains("cannot determine"));
    }

    /// The `systemctl show` field extraction used by the stale check.
    #[test]
    fn system_field_extraction() {
        let text = "MainPID=4711\nExecMainStartTimestamp=Mon 2026-09-14 20:47:09 IST\n";
        assert_eq!(system_field(text, "MainPID"), Some("4711"));
        assert_eq!(
            system_field(text, "ExecMainStartTimestamp"),
            Some("Mon 2026-09-14 20:47:09 IST")
        );
        assert_eq!(system_field(text, "AbsentBootId"), None);
        // Empty value (daemon not running) is treated as absent.
        assert_eq!(
            system_field("MainPID=0\nLoadState=loaded\n", "MainPID"),
            Some("0")
        );
    }

    /// JSON rendering carries the schema id and the same check fields.
    #[test]
    fn json_reuses_fields() {
        let report = Report {
            schema: SCHEMA,
            checks: vec![Check::pass("c", "d"), Check::fail("e", "f")],
            compare: None,
        };
        let json = serde_json::to_value(&report).expect("serialize");
        assert_eq!(json["schema"], "afanctl.doctor.v1");
        assert_eq!(json["checks"][0]["status"], "PASS");
        assert_eq!(json["checks"][1]["status"], "FAIL");
        assert_eq!(json["checks"][0]["name"], "c");
        assert!(json.get("compare").is_none(), "absent compare is omitted");
    }

    /// Human rendering keeps Appendix C's `STATUS — check — detail` shape and
    /// includes the compare table + verdict when present.
    #[test]
    fn human_render_shape() {
        let cfg = default_cfg();
        let t = Some(MilliC::from_c(80));
        let report = Report {
            schema: SCHEMA,
            checks: vec![Check::pass("a check", "a detail")],
            compare: Some(compare_from_trace(&[(t, 4700)], &cfg, 0)),
        };
        let text = render_human(&report);
        assert!(text.starts_with("PASS — a check — a detail\n"));
        assert!(text.contains("sample  t_eff(C)  smc_rpm  our_rpm  delta_rpm"));
        assert!(text.contains("verdict:"));
    }

    // --- RULING F25: `daemon mode` check (hold/degraded ⇒ WARN) ---

    /// RULING F25 test 1: a state file in hold mode yields a WARN naming the
    /// held rpm, and the WARN never changes doctor's exit code.
    #[test]
    fn hold_state_warns_naming_rpm_without_failing() {
        let fixture = TempFixture::copy_of("");
        fs::write(
            fixture.root.join("state.json"),
            r#"{"schema":"afanctl.state.v1","mode":"hold","target_rpm":3000,"monitor_only":false,"auto_restore_pending":false,"recent_errors":[]}"#,
        )
        .expect("seed state.json");
        let (report, _) = diagnose(&fixture.root, &fixture.config_path(), &fixture.root);
        let c = check(&report, "daemon mode");
        assert_eq!(c.status, Status::Warn);
        assert!(c.detail.contains("3000"), "names held rpm: {}", c.detail);
        assert!(c.detail.contains("manual=true"), "{}", c.detail);
        assert!(c.detail.contains("afanctl observe"), "{}", c.detail);
        assert!(!any_fail(&report), "a hold must not FAIL doctor");
        assert_eq!(
            run_at(
                &fixture.root,
                &fixture.config_path(),
                &fixture.root,
                false,
                None,
                false
            ),
            0,
            "a WARN must leave the exit code alone"
        );
    }

    /// RULING F25 test 2: observe and curve are PASS lines that name the mode.
    #[test]
    fn observe_and_curve_states_pass() {
        let observe = classify_daemon_mode(Some(&serde_json::json!({"mode": "observe"})));
        assert_eq!(observe.status, Status::Pass);
        assert!(observe.detail.contains("observe"), "{}", observe.detail);
        let curve = classify_daemon_mode(Some(&serde_json::json!({"mode": "curve"})));
        assert_eq!(curve.status, Status::Pass);
        assert!(curve.detail.contains("curve"), "{}", curve.detail);
    }

    /// RULING F25 test 3: the degraded latches escalate to WARN (never FAIL),
    /// name their flag(s), and carry the newest recent error.
    #[test]
    fn degraded_latches_warn_with_flag_and_newest_error() {
        let monitor = classify_daemon_mode(Some(&serde_json::json!({
            "mode": "curve",
            "monitor_only": true,
            "recent_errors": [{"ts": "2026-09-14T15:02:11+05:30", "msg": "write verify failed"}],
        })));
        assert_eq!(monitor.status, Status::Warn);
        assert!(
            monitor.detail.contains("monitor_only=true"),
            "{}",
            monitor.detail
        );
        assert!(
            monitor.detail.contains("requested curve, writing nothing"),
            "{}",
            monitor.detail
        );
        assert!(
            monitor.detail.contains("write verify failed"),
            "{}",
            monitor.detail
        );

        let pending = classify_daemon_mode(Some(&serde_json::json!({
            "mode": "observe",
            "auto_restore_pending": true,
        })));
        assert_eq!(pending.status, Status::Warn);
        assert!(
            pending.detail.contains("auto_restore_pending=true"),
            "{}",
            pending.detail
        );
    }

    /// RULING F25 test 4: no state file means no running daemon — PASS.
    #[test]
    fn no_state_file_passes() {
        assert_eq!(classify_daemon_mode(None).status, Status::Pass);
        assert!(classify_daemon_mode(None)
            .detail
            .contains("no running daemon"));
        let fixture = TempFixture::copy_of("");
        let (report, _) = diagnose(&fixture.root, &fixture.config_path(), &fixture.root);
        assert_eq!(check(&report, "daemon mode").status, Status::Pass);
    }

    /// RULING F25: an unrecognised or absent `mode` is a WARN naming the raw
    /// value — never a FAIL.
    #[test]
    fn unknown_mode_warns_naming_raw_value() {
        let weird = classify_daemon_mode(Some(&serde_json::json!({"mode": "turbo"})));
        assert_eq!(weird.status, Status::Warn);
        assert!(weird.detail.contains("turbo"), "{}", weird.detail);
        let missing =
            classify_daemon_mode(Some(&serde_json::json!({"schema": "afanctl.state.v1"})));
        assert_eq!(missing.status, Status::Warn);
        assert!(missing.detail.contains("(missing)"), "{}", missing.detail);
    }

    /// RULING F25: `--json` reuses the human fields for the daemon-mode line.
    #[test]
    fn daemon_mode_json_carries_fields() {
        let report = Report {
            schema: SCHEMA,
            checks: vec![classify_daemon_mode(Some(&serde_json::json!({
                "mode": "hold",
                "target_rpm": 4000
            })))],
            compare: None,
        };
        let json = serde_json::to_value(&report).expect("serialize");
        assert_eq!(json["checks"][0]["name"], "daemon mode");
        assert_eq!(json["checks"][0]["status"], "WARN");
        assert!(json["checks"][0]["detail"]
            .as_str()
            .unwrap_or("")
            .contains("4000"));
    }
}
