//! Hand-rolled CLI: parse the R5 grammar, dispatch verbs against the frozen
//! interfaces (DESIGN.md Appendix A), and format human/JSON output (Appendices
//! B/C). The only module allowed `println!`/`print!` (§8); no sysfs path string
//! or control logic lives here (`smc` owns paths, `supervisor` owns the loop).
//!
//! Exit codes (R11): 0 success, 1 runtime failure, 2 usage/CLI error. An unknown
//! verb/flag prints usage on stderr and exits 2. PHASE (b) wires `daemon`/
//! `once`/`hold` to the real supervisor: `daemon` builds a live `Supervisor`
//! (whose `run()` installs L2 + notifies READY); `once` runs exactly one
//! `step_once`; `--at-temp` simulates the sensor through an in-process
//! `MockSmc` (never sysfs); `--dry-run` computes a decision without writing
//! anything. The runtime dir for cmd/state files honours `AFANCTL_RUNTIME_DIR`
//! (Q-T6-2).

use std::path::PathBuf;

use crate::config::{Config, ResolvedConfig};
use crate::policy::{Controller, MilliC};
use crate::smc::{FanMode, FanState, MockSmc, SensorReading, Smc, SysfsSmc};
use crate::supervisor::{RunMode, RuntimePaths, StepReport, Supervisor};

/// Default config path (R6).
const DEFAULT_CONFIG: &str = "/etc/afanctl/afanctl.toml";
/// Default sysfs mount; `--sysfs-root` redirects all sysfs access (R1).
const DEFAULT_SYSFS_ROOT: &str = "/sys";
/// Runtime dir holding the plugin-facing cmd/state files (R8); see Q-T6-2.
const DEFAULT_RUNTIME_DIR: &str = "/run/afanctl";
/// Env override for the runtime dir (tests/dev only; see Q-T6-2).
const RUNTIME_DIR_ENV: &str = "AFANCTL_RUNTIME_DIR";

/// Global options accepted by every verb (R5).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Globals {
    config: PathBuf,
    sysfs_root: PathBuf,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            config: DEFAULT_CONFIG.into(),
            sysfs_root: DEFAULT_SYSFS_ROOT.into(),
        }
    }
}

/// `daemon --mode` value (R3); the startup default is observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonMode {
    Observe,
    Curve,
}

/// A fully parsed command line (grammar: R5).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Daemon {
        mode: DaemonMode,
    },
    Status {
        json: bool,
    },
    Doctor {
        json: bool,
        roundtrip: bool,
        compare_secs: Option<u64>,
    },
    Once {
        at_temp: Option<i32>,
        dry_run: bool,
        json: bool,
    },
    Observe,
    Curve,
    Hold {
        rpm: u32,
    },
    SelftestPanic,
    Version,
    Help,
}

/// CLI errors: `Usage` → exit 2, `Runtime` → exit 1 (R11 exit-code discipline).
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Runtime(String),
}

impl CliError {
    /// Process exit code for this error class (R11).
    fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage(_) => 2,
            CliError::Runtime(_) => 1,
        }
    }
}

/// Parse process arguments, dispatch the verb, return the exit code (R11).
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    run_with(&args)
}

/// Testable entry point: `args` excludes argv[0]. Every grammar violation
/// becomes usage text on stderr plus exit code 2 (never exit 0 on a bad flag).
fn run_with(args: &[String]) -> i32 {
    match parse(args) {
        Ok((command, globals)) => dispatch(command, &globals),
        Err(e) => {
            eprintln!("afanctl: {e}");
            if matches!(&e, CliError::Usage(_)) {
                eprintln!();
                eprint!("{}", usage());
            }
            e.exit_code()
        }
    }
}

/// Parse argv (without the program name) into a verb plus the globals in force.
/// `--config`/`--sysfs-root` are global: accepted before or after the verb.
fn parse(args: &[String]) -> Result<(Command, Globals), CliError> {
    let mut globals = Globals::default();
    let mut tokens: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => globals.config = PathBuf::from(value(args, &mut i, "--config")?),
            "--sysfs-root" => {
                globals.sysfs_root = PathBuf::from(value(args, &mut i, "--sysfs-root")?)
            }
            token => tokens.push(token),
        }
        i += 1;
    }
    let (verb, tail) = tokens.split_first().ok_or_else(|| bad("no verb given"))?;
    let command = match *verb {
        "--version" => {
            no_args(tail, "--version")?;
            Command::Version
        }
        "-h" | "--help" => {
            no_args(tail, "--help")?;
            Command::Help
        }
        v if v.starts_with('-') => return Err(bad(format!("unknown flag `{v}`"))),
        v => parse_verb(v, tail)?,
    };
    Ok((command, globals))
}

/// Parse one verb's flags; globals were already extracted by [`parse`].
fn parse_verb(verb: &str, tail: &[&str]) -> Result<Command, CliError> {
    let (mut json, mut roundtrip, mut compare, mut at_temp, mut dry_run) =
        (false, false, None, None, false);
    let mut mode = DaemonMode::Observe;
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < tail.len() {
        let flag = tail[i];
        match (verb, flag) {
            ("daemon", "--mode") => {
                let v = value(tail, &mut i, "--mode")?;
                mode = match v.as_str() {
                    "observe" => DaemonMode::Observe,
                    "curve" => DaemonMode::Curve,
                    other => {
                        return Err(bad(format!("--mode expects observe|curve, got `{other}`")))
                    }
                };
            }
            ("status" | "doctor" | "once", "--json") => json = true,
            ("doctor", "--roundtrip") => roundtrip = true,
            ("doctor", "--compare") => {
                compare = Some(parse_num(&value(tail, &mut i, "--compare")?, "--compare")?)
            }
            ("once", "--at-temp") => {
                at_temp = Some(parse_num(&value(tail, &mut i, "--at-temp")?, "--at-temp")?)
            }
            ("once", "--dry-run") => dry_run = true,
            (v, f) if f.starts_with('-') => return Err(bad(format!("{v}: unknown flag `{f}`"))),
            (_, p) => positional.push(p),
        }
        i += 1;
    }
    if verb != "hold" {
        if let Some(extra) = positional.first() {
            return Err(bad(format!("{verb}: unexpected argument `{extra}`")));
        }
    }
    match verb {
        "daemon" => Ok(Command::Daemon { mode }),
        "status" => Ok(Command::Status { json }),
        "doctor" => Ok(Command::Doctor {
            json,
            roundtrip,
            compare_secs: compare,
        }),
        "once" => Ok(Command::Once {
            at_temp,
            dry_run,
            json,
        }),
        "observe" => Ok(Command::Observe),
        "curve" => Ok(Command::Curve),
        "selftest-panic" => Ok(Command::SelftestPanic),
        "hold" => {
            let rpm = positional.first().ok_or_else(|| bad("hold needs an rpm"))?;
            if positional.len() > 1 {
                return Err(bad(format!(
                    "hold: unexpected argument `{}`",
                    positional[1]
                )));
            }
            if rpm.starts_with('-') {
                return Err(bad(format!("hold: rpm must be non-negative, got `{rpm}`")));
            }
            Ok(Command::Hold {
                rpm: parse_num(rpm, "hold rpm")?,
            })
        }
        v => Err(bad(format!("unknown verb `{v}`"))),
    }
}

/// Consume the value following a flag, advancing the cursor past it.
fn value<S: AsRef<str>>(args: &[S], at: &mut usize, flag: &str) -> Result<String, CliError> {
    let next = args
        .get(*at + 1)
        .ok_or_else(|| bad(format!("{flag} needs a value")))?;
    *at += 1;
    Ok(next.as_ref().to_string())
}

/// Reject any trailing argument (verbs that take none).
fn no_args(tail: &[&str], verb: &str) -> Result<(), CliError> {
    match tail.first() {
        Some(extra) => Err(bad(format!("{verb}: unexpected argument `{extra}`"))),
        None => Ok(()),
    }
}

/// Parse an integer flag value; a non-integer is a usage error.
fn parse_num<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, CliError> {
    s.parse()
        .map_err(|_| bad(format!("{what} expects an integer, got `{s}`")))
}

/// One-line construction of a usage error.
fn bad(msg: impl Into<String>) -> CliError {
    CliError::Usage(msg.into())
}

/// The `-h`/usage text; documents the exit codes (R11).
fn usage() -> String {
    format!(
        "\
afanctl {v} - applesmc fan supervisor (pre-T2 Intel Mac)

USAGE:  afanctl <verb> [options]

VERBS:
  daemon [--mode observe|curve]     supervisor loop (default observe)
  status [--json]                   temps, t_eff, fan, mode, config provenance
  doctor [--json] [--roundtrip] [--compare <s>]   diagnostics (exit 1 on FAIL)
  once [--at-temp <C>] [--dry-run] [--json]   one control iteration, print the decision
        --at-temp <C>  simulate the sensor at C (in-process mock; never touches sysfs)
        --dry-run      compute the decision but write nothing (no cmd/state file)
  observe | curve | hold <rpm>      write the command file (R8; daemon clamps)
  selftest-panic                    hidden: deliberate panic to prove L2

GLOBALS:
  --config <path>      config TOML (default {cfg})
  --sysfs-root <dir>   sysfs root (default {sysfs})
  --version, -h        version / this help

EXIT CODES: 0 success   1 runtime failure   2 usage/CLI error
",
        v = env!("CARGO_PKG_VERSION"),
        cfg = DEFAULT_CONFIG,
        sysfs = DEFAULT_SYSFS_ROOT,
    )
}

fn dispatch(command: Command, globals: &Globals) -> i32 {
    match command {
        Command::Version => {
            println!("afanctl {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Command::Help => {
            print!("{}", usage());
            0
        }
        Command::Daemon { mode } => run_daemon(mode, globals),
        Command::Status { json } => run_status(json, globals),
        Command::Doctor {
            json,
            roundtrip,
            compare_secs,
        } => run_doctor(json, roundtrip, compare_secs),
        Command::Once {
            at_temp,
            dry_run,
            json,
        } => run_once(at_temp, dry_run, json, globals),
        Command::Observe => write_cmd("observe", None),
        Command::Curve => write_cmd("curve", None),
        Command::Hold { rpm } => run_hold(rpm, globals),
        Command::SelftestPanic => crate::safety::arm_test_panic(),
    }
}

/// Print a runtime error on stderr and return exit code 1.
fn fail(err: impl std::fmt::Display) -> i32 {
    eprintln!("afanctl: {err}");
    1
}

/// Runtime dir for cmd/state files (Q-T6-2: env-overridable for tests/dev).
fn runtime_dir() -> PathBuf {
    std::env::var_os(RUNTIME_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_RUNTIME_DIR))
}

/// Load the config (missing file → defaults); unknown keys are warnings.
fn load_config(path: &std::path::Path) -> Result<(Config, Vec<String>), CliError> {
    let (config, warnings) = Config::load(path).map_err(|e| CliError::Runtime(e.to_string()))?;
    for warning in &warnings {
        tracing::warn!(warning = %warning, "config");
    }
    Ok((config, warnings))
}

/// cmd/state file locations from the env-overridable runtime dir (Q-T6-2).
fn runtime_paths() -> RuntimePaths {
    let dir = runtime_dir();
    RuntimePaths {
        cmd: dir.join("cmd.json"),
        state: dir.join("state.json"),
    }
}

/// Wire a supervisor over any backend. `Supervisor::new` validates the config
/// band against the backend's hardware `(min, max)` rpm; `run()` installs L2
/// and notifies READY.
fn build_supervisor_with(
    smc: Box<dyn Smc>,
    config: &Config,
    start: RunMode,
) -> Result<Supervisor, CliError> {
    Supervisor::new(smc, &ResolvedConfig::from(config), start, &runtime_paths())
        .map_err(|e| CliError::Runtime(e.to_string()))
}

/// Build a live supervisor over the real sysfs backend (`--sysfs-root`).
fn build_supervisor(globals: &Globals, start: RunMode) -> Result<Supervisor, CliError> {
    let (config, _warnings) = load_config(&globals.config)?;
    let smc = SysfsSmc::open(&globals.sysfs_root).map_err(|e| CliError::Runtime(e.to_string()))?;
    build_supervisor_with(Box::new(smc), &config, start)
}

fn run_daemon(mode: DaemonMode, globals: &Globals) -> i32 {
    let start = match mode {
        DaemonMode::Observe => RunMode::Observe,
        DaemonMode::Curve => RunMode::Curve,
    };
    match build_supervisor(globals, start) {
        // `run()` arms L2 (pre-opened fd), notifies READY, then loops forever.
        Ok(mut supervisor) => supervisor.run(),
        Err(e) => fail(e),
    }
}

/// One simulated sensor reading at whole °C (the `--at-temp` input).
fn simulated_reading(c: i32) -> SensorReading {
    SensorReading {
        label: "simulated (--at-temp)".to_string(),
        milli_c: Some(MilliC::from_c(c)),
    }
}

/// `once`: exactly one control iteration, print the decision line, exit.
///
/// `--at-temp` routes through an in-process `MockSmc` seeded from the config
/// band, so it never opens `--sysfs-root` (even when that flag is set).
/// `--dry-run` computes the controller decision without touching any backend
/// and never writes a state file.
fn run_once(at_temp: Option<i32>, dry_run: bool, json: bool, globals: &Globals) -> i32 {
    tracing::debug!(at_temp = ?at_temp, dry_run, json, "once");
    let (config, _warnings) = match load_config(&globals.config) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let report = if dry_run {
        match dry_run_report(at_temp, &config, globals) {
            Ok(r) => r,
            Err(e) => return fail(e),
        }
    } else {
        match step_once_report(at_temp, &config, globals) {
            Ok(r) => r,
            Err(e) => return fail(e),
        }
    };
    if json {
        println!("{}", once_json(&report));
    } else {
        println!("{}", format_once(&report));
    }
    0
}

/// Live one-shot: a real `Supervisor::step_once` over the sysfs backend, or
/// (with `--at-temp`) over a `MockSmc` whose band is the configured curve band.
fn step_once_report(
    at_temp: Option<i32>,
    config: &Config,
    globals: &Globals,
) -> Result<StepReport, CliError> {
    let mut supervisor = match at_temp {
        Some(c) => {
            let mut mock = MockSmc::new(config.min_rpm, config.max_rpm);
            mock.set_sensors(vec![simulated_reading(c)]);
            build_supervisor_with(Box::new(mock), config, RunMode::Curve)?
        }
        None => {
            let smc = SysfsSmc::open(&globals.sysfs_root)
                .map_err(|e| CliError::Runtime(e.to_string()))?;
            build_supervisor_with(Box::new(smc), config, RunMode::Curve)?
        }
    };
    Ok(supervisor.step_once())
}

/// Simulated one-shot: the pure controller step only — no smc actuation, no
/// state file (PRD R5 `--dry-run`). Sensor input is synthesized when
/// `--at-temp` is present, otherwise read (read-only) from the backend.
fn dry_run_report(
    at_temp: Option<i32>,
    config: &Config,
    globals: &Globals,
) -> Result<StepReport, CliError> {
    let readings = match at_temp {
        Some(c) => vec![simulated_reading(c)],
        None => {
            let smc = SysfsSmc::open(&globals.sysfs_root)
                .map_err(|e| CliError::Runtime(e.to_string()))?;
            smc.read_sensors()
                .map_err(|e| CliError::Runtime(e.to_string()))?
        }
    };
    let mut controller = Controller::new(&ResolvedConfig::from(config));
    let decision = controller.step_curve(&readings);
    Ok(StepReport {
        mode: RunMode::Curve,
        t_eff: controller.t_eff(),
        decision,
        applied_rpm: None,
        verified: false,
        notes: vec!["dry-run: decision only, no smc writes".to_string()],
    })
}

/// Human rendering of one poll report (decision line, Appendix B field names).
fn format_once(report: &StepReport) -> String {
    let applied = report
        .applied_rpm
        .map_or_else(|| "none".to_string(), |rpm| rpm.to_string());
    let t_eff = report.t_eff.map_or_else(|| "n/a".to_string(), temp_c);
    let mut line = format!(
        "mode={} t_eff={t_eff}C decision={:?} applied_rpm={applied} verified={}",
        mode_token(&report.mode),
        report.decision,
        report.verified
    );
    if !report.notes.is_empty() {
        line.push_str(&format!(" notes=[{}]", report.notes.join("; ")));
    }
    line
}

/// JSON rendering of one poll report (`once --json`); same fields as the line.
fn once_json(report: &StepReport) -> serde_json::Value {
    serde_json::json!({
        "mode": mode_token(&report.mode),
        "t_eff_c": report.t_eff.map(|m| m.0 as f64 / 1000.0),
        "decision": format!("{:?}", report.decision),
        "applied_rpm": report.applied_rpm,
        "verified": report.verified,
        "notes": report.notes,
    })
}

/// Schema-token mode name (Appendix B `mode` values).
fn mode_token(mode: &RunMode) -> &'static str {
    match mode {
        RunMode::Observe => "observe",
        RunMode::Curve => "curve",
        RunMode::Hold(_) => "hold",
    }
}

/// `doctor`: T7 owns the checklist/`--compare` output (Appendix C). Per ruling on
/// D-T6-1 (ACCEPTED), `doctor::run` takes `json` and renders Appendix C as JSON
/// (same fields) when set. The T7 stub `unimplemented!()` panics: a panic must
/// never escape as a raw exit 101, so it is caught here and mapped to a clear
/// stderr line plus exit 1. T7 replaces the body; the catch stays harmless.
fn run_doctor(json: bool, roundtrip: bool, compare_secs: Option<u64>) -> i32 {
    match std::panic::catch_unwind(|| crate::doctor::run(roundtrip, compare_secs, json)) {
        Ok(code) => code,
        Err(_) => {
            eprintln!("doctor: not available yet (T7 pending)");
            1
        }
    }
}

/// `hold`: clamp/reject the requested rpm against the hardware band read at
/// discovery (`fan1_min`/`fan1_max` via `SysfsSmc::open`, PRD R5/Q2) BEFORE
/// writing `cmd.json`. Below `fan1_min` is rejected with exit 1 (a fan cannot
/// spin slower than its hardware floor); above `fan1_max` is clamped down.
fn run_hold(rpm: u32, globals: &Globals) -> i32 {
    let smc = match SysfsSmc::open(&globals.sysfs_root) {
        Ok(v) => v,
        Err(e) => return fail(CliError::Runtime(e.to_string())),
    };
    let (hw_min, hw_max) = (smc.hw_min_rpm(), smc.hw_max_rpm());
    if rpm < hw_min {
        return fail(CliError::Runtime(format!(
            "invalid hold rpm: {rpm} is below fan1_min {hw_min} (fix: hold a value in {hw_min}..={hw_max} rpm)"
        )));
    }
    let target = rpm.min(hw_max);
    if target != rpm {
        tracing::info!(rpm, target, hw_max, "hold rpm clamped to fan1_max");
    }
    write_cmd("hold", Some(target))
}

/// `observe`/`curve`: write `cmd.json` atomically (R8). The daemon owns all
/// sysfs writes and re-validates/clamps; the CLI never touches sysfs.
fn write_cmd(mode: &str, rpm: Option<u32>) -> i32 {
    // Field order matches Appendix B's `afanctl.cmd.v1` example exactly.
    let body = match rpm {
        Some(rpm) => format!(r#"{{"schema":"afanctl.cmd.v1","mode":"{mode}","rpm":{rpm}}}"#),
        None => format!(r#"{{"schema":"afanctl.cmd.v1","mode":"{mode}"}}"#),
    };
    match write_atomic(&runtime_dir().join("cmd.json"), &body) {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}

/// Atomic tmp+rename write (R8); never writes sysfs, `/run` (or override) only.
fn write_atomic(path: &std::path::Path, body: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{}: no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
}

/// Merged status snapshot: config + live sysfs reads + the daemon's state file
/// when present (PRD R7). `state = None` means the daemon is not running.
struct StatusData {
    sensors: Vec<SensorReading>,
    fan: FanState,
    hw_min: u32,
    hw_max: u32,
    t_eff: Option<MilliC>,
    state: Option<serde_json::Value>,
    source: String,
    config: Config,
}

/// Gather the merged status: config provenance + direct sysfs reads (fan
/// rpm/mode work even with the daemon down) + the state file if published.
fn gather_status(globals: &Globals) -> Result<StatusData, CliError> {
    let (config, _warnings) = load_config(&globals.config)?;
    let smc = SysfsSmc::open(&globals.sysfs_root).map_err(|e| CliError::Runtime(e.to_string()))?;
    let sensors = smc
        .read_sensors()
        .map_err(|e| CliError::Runtime(e.to_string()))?;
    let fan = smc
        .read_fan()
        .map_err(|e| CliError::Runtime(e.to_string()))?;
    let (hw_min, hw_max) = (smc.hw_min_rpm(), smc.hw_max_rpm());
    let t_eff = sensors.iter().filter_map(|s| s.milli_c).max_by_key(|m| m.0);
    let source = if globals.config.exists() {
        globals.config.display().to_string()
    } else {
        "defaults".to_string()
    };
    Ok(StatusData {
        sensors,
        fan,
        hw_min,
        hw_max,
        t_eff,
        state: read_state(),
        source,
        config,
    })
}

/// Appendix B `afanctl.status.v1` rendering of a status snapshot.
fn status_json(d: &StatusData) -> serde_json::Value {
    let temp = |m: MilliC| serde_json::json!(m.0 as f64 / 1000.0);
    let field = |key: &str| d.state.as_ref().and_then(|s| s.get(key));
    let running = d.state.is_some();
    serde_json::json!({
        "schema": "afanctl.status.v1",
        "daemon": {"running": running, "mode": field("mode").and_then(|m| m.as_str()).unwrap_or("observe"), "watchdog_armed": running, "uptime_s": field("watchdog_pings").and_then(|p| p.as_u64()).unwrap_or(0)},
        "sensors": d.sensors.iter().map(|s| serde_json::json!({"label": s.label, "temp_c": s.milli_c.map_or(serde_json::Value::Null, temp)})).collect::<Vec<_>>(),
        "effective": {"temp_c": d.t_eff.map_or(serde_json::Value::Null, temp), "method": "max"},
        "fan": {"rpm": d.fan.rpm, "min_rpm": d.hw_min, "max_rpm": d.hw_max, "target_rpm": field("target_rpm").cloned().unwrap_or(serde_json::Value::Null), "manual": d.fan.mode == FanMode::Manual},
        "config": {"high_c": d.config.high_c, "max_c": d.config.max_c, "min_rpm": d.config.min_rpm, "max_rpm": d.config.max_rpm, "interval_s": d.config.interval_s, "source": d.source},
        "recent_errors": field("recent_errors").cloned().unwrap_or(serde_json::Value::Array(Vec::new())),
    })
}

/// `status`: print the merged snapshot (R7) as Appendix B JSON (`--json`) or a
/// compact human summary. A missing state file reads as `daemon: not running`.
fn run_status(json: bool, globals: &Globals) -> i32 {
    let d = match gather_status(globals) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    if json {
        println!("{}", status_json(&d));
        return 0;
    }
    print!("{}", status_human(&d));
    0
}

/// Human status summary (one line per fact); `temp_c` already carries the
/// one-decimal format, so it is interpolated directly (never `{:.1}` again).
fn status_human(d: &StatusData) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "daemon: {}\n",
        if d.state.is_some() {
            "running"
        } else {
            "not running"
        }
    ));
    for sensor in &d.sensors {
        let reading = sensor
            .milli_c
            .map_or_else(|| "n/a".to_string(), |m| format!("{} C", temp_c(m)));
        out.push_str(&format!("sensor {}: {reading}\n", sensor.label));
    }
    out.push_str(&format!(
        "t_eff: {}\n",
        d.t_eff
            .map_or_else(|| "n/a".to_string(), |m| format!("{} C", temp_c(m)))
    ));
    out.push_str(&format!(
        "fan: {} rpm ({}..{}, manual={})\n",
        d.fan.rpm,
        d.hw_min,
        d.hw_max,
        d.fan.mode == FanMode::Manual
    ));
    out.push_str(&format!(
        "config: high={} max={} min_rpm={} max_rpm={} interval_s={} source={}\n",
        d.config.high_c,
        d.config.max_c,
        d.config.min_rpm,
        d.config.max_rpm,
        d.config.interval_s,
        d.source
    ));
    out
}

/// Whole °C from `MilliC` for display (milli-°C is the internal unit; §7).
fn temp_c(m: MilliC) -> String {
    format!("{:.1}", m.0 as f64 / 1000.0)
}

/// Read `state.json` if the daemon has published one (R7); absence = not running.
fn read_state() -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(runtime_dir().join("state.json")).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<(Command, Globals), CliError> {
        parse(&args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
    }

    fn command(args: &[&str]) -> Command {
        parse_args(args).expect("parse should succeed").0
    }

    fn error(args: &[&str]) -> CliError {
        parse_args(args).expect_err("parse should fail")
    }

    #[test]
    fn parses_every_verb() {
        let cases: Vec<(Vec<&str>, Command)> = vec![
            (
                vec!["daemon"],
                Command::Daemon {
                    mode: DaemonMode::Observe,
                },
            ),
            (
                vec!["daemon", "--mode", "curve"],
                Command::Daemon {
                    mode: DaemonMode::Curve,
                },
            ),
            (vec!["status"], Command::Status { json: false }),
            (vec!["status", "--json"], Command::Status { json: true }),
            (
                vec!["doctor"],
                Command::Doctor {
                    json: false,
                    roundtrip: false,
                    compare_secs: None,
                },
            ),
            (
                vec!["doctor", "--json", "--roundtrip", "--compare", "30"],
                Command::Doctor {
                    json: true,
                    roundtrip: true,
                    compare_secs: Some(30),
                },
            ),
            (
                vec!["once"],
                Command::Once {
                    at_temp: None,
                    dry_run: false,
                    json: false,
                },
            ),
            (
                vec!["once", "--at-temp", "80", "--dry-run"],
                Command::Once {
                    at_temp: Some(80),
                    dry_run: true,
                    json: false,
                },
            ),
            (
                vec!["once", "--at-temp", "-5"],
                Command::Once {
                    at_temp: Some(-5),
                    dry_run: false,
                    json: false,
                },
            ),
            (
                vec!["once", "--json"],
                Command::Once {
                    at_temp: None,
                    dry_run: false,
                    json: true,
                },
            ),
            (vec!["observe"], Command::Observe),
            (vec!["curve"], Command::Curve),
            (vec!["hold", "3000"], Command::Hold { rpm: 3000 }),
            (vec!["selftest-panic"], Command::SelftestPanic),
            (vec!["--version"], Command::Version),
            (vec!["-h"], Command::Help),
            (vec!["--help"], Command::Help),
        ];
        for (args, expected) in &cases {
            assert_eq!(&command(args), expected, "{args:?}");
        }
    }

    #[test]
    fn globals_are_accepted_anywhere() {
        let (_, globals) = parse_args(&["--config", "/etc/x.toml", "status"]).expect("ok");
        assert_eq!(globals.config, PathBuf::from("/etc/x.toml"));
        let (_, globals) = parse_args(&["status", "--sysfs-root", "/tmp/fixtures"]).expect("ok");
        assert_eq!(globals.sysfs_root, PathBuf::from("/tmp/fixtures"));
        let (_, globals) =
            parse_args(&["hold", "2500", "--config", "a.toml", "--sysfs-root", "b"]).expect("ok");
        assert_eq!(globals.config, PathBuf::from("a.toml"));
        assert_eq!(globals.sysfs_root, PathBuf::from("b"));
        let (command, _) = parse_args(&["--config", "a", "daemon", "--mode", "curve"]).expect("ok");
        assert_eq!(
            command,
            Command::Daemon {
                mode: DaemonMode::Curve
            }
        );
    }

    #[test]
    fn usage_errors_exit_2() {
        let cases: Vec<Vec<&str>> = vec![
            vec![],
            vec!["frobnicate"],
            vec!["--bogus"],
            vec!["-x"],
            vec!["--config"],
            vec!["--sysfs-root"],
            vec!["daemon", "--mode"],
            vec!["daemon", "--mode", "turbo"],
            vec!["daemon", "--bogus"],
            vec!["status", "extra"],
            vec!["status", "--compare", "5"],
            vec!["doctor", "--compare"],
            vec!["doctor", "--compare", "soon"],
            vec!["doctor", "--bogus"],
            vec!["once", "--at-temp"],
            vec!["once", "--at-temp", "warm"],
            vec!["once", "--bogus"],
            vec!["observe", "--json"],
            vec!["curve", "extra"],
            vec!["hold"],
            vec!["hold", "abc"],
            vec!["hold", "-1"],
            vec!["hold", "3000", "extra"],
            vec!["selftest-panic", "extra"],
            vec!["--version", "extra"],
            vec!["-h", "extra"],
        ];
        for args in &cases {
            let e = error(args);
            assert!(
                matches!(e, CliError::Usage(_)),
                "expected usage error for {args:?}"
            );
            assert_eq!(e.exit_code(), 2, "exit code for {args:?}");
        }
    }

    #[test]
    fn exit_codes_and_version_output() {
        assert_eq!(CliError::Usage("x".into()).exit_code(), 2);
        assert_eq!(CliError::Runtime("x".into()).exit_code(), 1);
        assert_eq!(run_with(&["--version".to_string()]), 0);
        assert_eq!(run_with(&["--help".to_string()]), 0);
        assert_eq!(run_with(&["--bogus".to_string()]), 2);
        assert_eq!(run_with(&[]), 2);
        let text = usage();
        assert!(text.contains("afanctl"));
        assert!(text.contains("EXIT CODES"));
        assert!(text.contains("0 success   1 runtime failure   2 usage/CLI error"));
    }

    // ---- PHASE (b) wiring: fixtures/mocks only, never real sysfs ----

    /// Serializes `AFANCTL_RUNTIME_DIR` mutations (process-global, parallel tests).
    fn runtime_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn next_seq() -> u32 {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Private tempdir for cmd/state files (never `/run`).
    struct RuntimeDir(PathBuf);
    impl RuntimeDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "afanctl-t6b-{}-{}",
                std::process::id(),
                next_seq()
            ));
            std::fs::create_dir_all(&dir).expect("tempdir");
            Self(dir)
        }
        fn cmd(&self) -> PathBuf {
            self.0.join("cmd.json")
        }
        fn state(&self) -> PathBuf {
            self.0.join("state.json")
        }
    }
    impl Drop for RuntimeDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Sets `AFANCTL_RUNTIME_DIR` for the test's duration while holding the
    /// env lock; clears it on drop even if an assertion panics.
    struct RuntimeEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
        dir: RuntimeDir,
    }
    impl RuntimeEnv {
        fn new() -> Self {
            let lock = runtime_lock();
            let dir = RuntimeDir::new();
            std::env::set_var(RUNTIME_DIR_ENV, &dir.0);
            Self { _lock: lock, dir }
        }
        fn cmd(&self) -> PathBuf {
            self.dir.cmd()
        }
        fn state(&self) -> PathBuf {
            self.dir.state()
        }
    }
    impl Drop for RuntimeEnv {
        fn drop(&mut self) {
            std::env::remove_var(RUNTIME_DIR_ENV);
        }
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs")
    }

    fn globals(sysfs: &std::path::Path) -> Globals {
        Globals {
            config: PathBuf::from("/nonexistent/afanctl/afanctl.toml"),
            sysfs_root: sysfs.to_path_buf(),
        }
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn read_json(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("file exists")).expect("json")
    }

    /// DONE-WHEN integration-style check: the CLI wiring builds a real
    /// `Supervisor` over an in-process `MockSmc` and one poll yields a verified
    /// `SetSpeed` plus a published `state.json` — no binary spawn.
    #[test]
    fn daemon_wiring_builds_real_supervisor_over_mock() {
        let env = RuntimeEnv::new();
        let mut mock = MockSmc::new(1200, 7200);
        mock.set_sensors(vec![simulated_reading(80)]);
        let mut supervisor =
            build_supervisor_with(Box::new(mock), &Config::defaults(), RunMode::Curve)
                .expect("supervisor builds over the mock");

        let report = supervisor.step_once();
        assert_eq!(report.mode, RunMode::Curve);
        assert_eq!(report.t_eff, Some(MilliC(80_000)));
        assert_eq!(report.decision, crate::policy::Decision::SetSpeed(1950));
        assert_eq!(report.applied_rpm, Some(1950));
        assert!(report.verified);

        let state = read_json(&env.state());
        assert_eq!(state["schema"], "afanctl.state.v1");
        assert_eq!(state["mode"], "curve");
        assert_eq!(state["last_written_rpm"], 1950);
    }

    /// `once --at-temp` must not open `--sysfs-root` even when it is set.
    #[test]
    fn once_at_temp_never_touches_sysfs() {
        let _env = RuntimeEnv::new();
        let code = run_with(&args(&[
            "once",
            "--at-temp",
            "80",
            "--sysfs-root",
            "/nonexistent-sysfs-root",
        ]));
        assert_eq!(
            code, 0,
            "at-temp is in-process simulation, not a sysfs read"
        );
    }

    /// `once --dry-run` computes a decision but writes no cmd/state file.
    #[test]
    fn once_dry_run_writes_nothing() {
        let env = RuntimeEnv::new();
        let code = run_with(&args(&["once", "--at-temp", "80", "--dry-run"]));
        assert_eq!(code, 0);
        assert!(!env.state().exists(), "dry-run must not write state.json");
        assert!(!env.cmd().exists(), "dry-run must not write cmd.json");
    }

    /// `once --dry-run` without `--at-temp` reads the backend read-only.
    #[test]
    fn once_dry_run_reads_fixture_without_writing() {
        let env = RuntimeEnv::new();
        let fixture = fixture_root().display().to_string();
        let code = run_with(&args(&["once", "--dry-run", "--sysfs-root", &fixture]));
        assert_eq!(code, 0);
        assert!(!env.state().exists(), "dry-run must not write state.json");
    }

    /// `once --json` report shape (Appendix B field names).
    #[test]
    fn once_json_reports_the_decision_fields() {
        let report = StepReport {
            mode: RunMode::Hold(3000),
            t_eff: Some(MilliC(72_500)),
            decision: crate::policy::Decision::SetSpeed(3000),
            applied_rpm: Some(3000),
            verified: true,
            notes: vec!["cmd applied: hold 3000".to_string()],
        };
        let value = once_json(&report);
        assert_eq!(value["mode"], "hold");
        assert_eq!(value["t_eff_c"], 72.5);
        assert_eq!(value["decision"], "SetSpeed(3000)");
        assert_eq!(value["applied_rpm"], 3000);
        assert_eq!(value["verified"], true);
        assert_eq!(value["notes"][0], "cmd applied: hold 3000");
    }

    /// Human decision line: fixed fields plus notes when present.
    #[test]
    fn format_once_renders_the_decision_line() {
        let report = StepReport {
            mode: RunMode::Curve,
            t_eff: Some(MilliC(80_000)),
            decision: crate::policy::Decision::SetSpeed(1950),
            applied_rpm: Some(1950),
            verified: true,
            notes: Vec::new(),
        };
        assert_eq!(
            format_once(&report),
            "mode=curve t_eff=80.0C decision=SetSpeed(1950) applied_rpm=1950 verified=true"
        );
        let dry = StepReport {
            mode: RunMode::Curve,
            t_eff: Some(MilliC(80_000)),
            decision: crate::policy::Decision::SetSpeed(1950),
            applied_rpm: None,
            verified: false,
            notes: vec!["dry-run: decision only, no smc writes".to_string()],
        };
        let line = format_once(&dry);
        assert!(line.contains("applied_rpm=none"));
        assert!(line.contains("notes=[dry-run: decision only, no smc writes]"));
    }

    /// `hold` below `fan1_min` → exit 1, key+fix message, no cmd.json written.
    #[test]
    fn hold_rejects_below_fan1_min_and_writes_no_cmd() {
        let env = RuntimeEnv::new();
        let fixture = fixture_root().display().to_string();
        let code = run_with(&args(&["hold", "300", "--sysfs-root", &fixture]));
        assert_eq!(code, 1, "below fan1_min is a runtime failure (R5/Q2)");
        assert!(!env.cmd().exists(), "a rejected hold never writes cmd.json");
    }

    /// `hold` above `fan1_max` → clamped down, exit 0.
    #[test]
    fn hold_clamps_above_fan1_max() {
        let env = RuntimeEnv::new();
        let fixture = fixture_root().display().to_string();
        let code = run_with(&args(&["hold", "99999", "--sysfs-root", &fixture]));
        assert_eq!(code, 0);
        let body = read_json(&env.cmd());
        assert_eq!(body["schema"], "afanctl.cmd.v1");
        assert_eq!(body["mode"], "hold");
        assert_eq!(body["rpm"], 7200, "clamped to fan1_max");
    }

    /// `hold` within the hardware band → written unchanged, exit 0.
    #[test]
    fn hold_within_range_writes_exact_rpm() {
        let env = RuntimeEnv::new();
        let fixture = fixture_root().display().to_string();
        let code = run_with(&args(&["hold", "3000", "--sysfs-root", &fixture]));
        assert_eq!(code, 0);
        assert_eq!(read_json(&env.cmd())["rpm"], 3000);
    }

    /// `status` merges config + state file + direct sysfs reads (R7).
    #[test]
    fn status_merges_state_file_and_sysfs_reads() {
        let env = RuntimeEnv::new();
        std::fs::write(
            env.state(),
            r#"{"schema":"afanctl.state.v1","mode":"curve","target_rpm":3400,"watchdog_pings":42,"recent_errors":[{"ts":"t","msg":"x"}]}"#,
        )
        .expect("seed state.json");

        let data = gather_status(&globals(&fixture_root())).expect("gather");
        let value = status_json(&data);

        assert_eq!(value["schema"], "afanctl.status.v1");
        assert_eq!(value["daemon"]["running"], true);
        assert_eq!(value["daemon"]["mode"], "curve");
        assert_eq!(value["daemon"]["uptime_s"], 42);
        assert_eq!(value["fan"]["rpm"], 1200, "live sysfs read");
        assert_eq!(value["fan"]["min_rpm"], 1200);
        assert_eq!(value["fan"]["max_rpm"], 7200);
        assert_eq!(value["fan"]["target_rpm"], 3400, "from the state file");
        assert_eq!(value["fan"]["manual"], false);
        assert_eq!(
            value["effective"]["temp_c"], 45.0,
            "max over fixture sensors"
        );
        assert_eq!(value["config"]["source"], "defaults");
        assert_eq!(value["sensors"].as_array().map(Vec::len), Some(3));
    }

    /// Missing state file → `daemon: not running`, sysfs reads still merged.
    #[test]
    fn status_missing_state_file_reports_not_running() {
        let _env = RuntimeEnv::new();
        let data = gather_status(&globals(&fixture_root())).expect("gather");
        let value = status_json(&data);
        assert_eq!(value["daemon"]["running"], false);
        assert_eq!(value["daemon"]["mode"], "observe");
        assert_eq!(
            value["fan"]["rpm"], 1200,
            "fan readable with the daemon down"
        );
        assert_eq!(value["effective"]["temp_c"], 45.0);
    }

    /// Human status must render whole temps with one decimal (45.0 C, not 4 C).
    #[test]
    fn status_human_renders_full_temperatures() {
        let _env = RuntimeEnv::new();
        let data = gather_status(&globals(&fixture_root())).expect("gather");
        let text = status_human(&data);
        assert!(text.contains("daemon: not running"), "{text}");
        assert!(text.contains("sensor Package id 0: 45.0 C"), "{text}");
        assert!(text.contains("t_eff: 45.0 C"), "{text}");
        assert!(
            text.contains("fan: 1200 rpm (1200..7200, manual=false)"),
            "{text}"
        );
    }

    /// The T7 stub's panic maps to a clear exit 1, never a raw 101.
    #[test]
    fn doctor_stub_maps_panic_to_exit_1() {
        assert_eq!(run_doctor(false, false, None), 1);
        assert_eq!(run_doctor(true, true, Some(1)), 1);
    }

    #[test]
    fn usage_documents_simulation_flags() {
        let text = usage();
        assert!(text.contains("--at-temp"));
        assert!(text.contains("never touches sysfs"));
        assert!(text.contains("--dry-run"));
    }
}
