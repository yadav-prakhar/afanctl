//! Hand-rolled CLI: parse the R5 grammar, dispatch verbs against the frozen
//! interfaces (DESIGN.md Appendix A), and format human/JSON output (Appendices
//! B/C). The only module allowed `println!`/`print!` (§8); no sysfs path string
//! or control logic lives here (`smc` owns paths, `supervisor` owns the loop).
//!
//! Exit codes (R11): 0 success, 1 runtime failure, 2 usage/CLI error. An unknown
//! verb/flag prints usage on stderr and exits 2. PHASE (a): `daemon`/`once`/
//! `status` reach the T5/T1/T3 stubs and fail loudly until PHASE (b) wires them.
//! The runtime dir for cmd/state files honours `AFANCTL_RUNTIME_DIR` (Q-T6-2).

use std::path::PathBuf;

use crate::config::{Config, ResolvedConfig};
use crate::policy::MilliC;
use crate::smc::{FanMode, Smc, SysfsSmc};
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
            ("status" | "doctor", "--json") => json = true,
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
        "once" => Ok(Command::Once { at_temp, dry_run }),
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
  once [--at-temp <C>] [--dry-run]  one control iteration, print the decision
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
        Command::Once { at_temp, dry_run } => run_once(at_temp, dry_run, globals),
        Command::Observe => write_cmd("observe", None),
        Command::Curve => write_cmd("curve", None),
        Command::Hold { rpm } => write_cmd("hold", Some(rpm)),
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

/// Build a live supervisor over the real sysfs backend. PHASE (a): T1/T3/T5
/// bodies are stubs, so this fails loudly until those tasks merge.
fn build_supervisor(globals: &Globals, start: RunMode) -> Result<Supervisor, CliError> {
    let (config, warnings) =
        Config::load(&globals.config).map_err(|e| CliError::Runtime(e.to_string()))?;
    for warning in warnings {
        tracing::warn!(warning = %warning, "config");
    }
    let smc = SysfsSmc::open(&globals.sysfs_root).map_err(|e| CliError::Runtime(e.to_string()))?;
    let dir = runtime_dir();
    let paths = RuntimePaths {
        cmd: dir.join("cmd.json"),
        state: dir.join("state.json"),
    };
    Supervisor::new(Box::new(smc), &ResolvedConfig::from(&config), start, &paths)
        .map_err(|e| CliError::Runtime(e.to_string()))
}

fn run_daemon(mode: DaemonMode, globals: &Globals) -> i32 {
    let start = match mode {
        DaemonMode::Observe => RunMode::Observe,
        DaemonMode::Curve => RunMode::Curve,
    };
    match build_supervisor(globals, start) {
        Ok(mut supervisor) => supervisor.run(),
        Err(e) => fail(e),
    }
}

/// `once`: one control iteration. PHASE (b) routes `--at-temp` (simulated
/// sensor input) and `--dry-run` through the real `StepReport`.
fn run_once(at_temp: Option<i32>, dry_run: bool, globals: &Globals) -> i32 {
    tracing::debug!(at_temp = ?at_temp, dry_run = dry_run, "once (simulation: PHASE b)");
    match build_supervisor(globals, RunMode::Curve) {
        Ok(mut supervisor) => {
            println!("{}", format_once(&supervisor.step_once()));
            0
        }
        Err(e) => fail(e),
    }
}

/// Human rendering of one poll report (extended in PHASE b against real data).
fn format_once(report: &StepReport) -> String {
    let mode = match &report.mode {
        RunMode::Observe => "observe",
        RunMode::Curve => "curve",
        RunMode::Hold(_) => "hold",
    };
    let applied = report
        .applied_rpm
        .map_or_else(|| "none".into(), |rpm| rpm.to_string());
    let t_eff = report.t_eff.map_or_else(|| "n/a".into(), temp_c);
    format!(
        "mode={mode} t_eff={t_eff}C decision={:?} applied_rpm={applied} verified={}",
        report.decision, report.verified
    )
}

/// `doctor`: T7 owns the checklist/`--compare` output (Appendix C). Per ruling on
/// D-T6-1 (ACCEPTED), `doctor::run` takes `json` and renders Appendix C as JSON
/// (same fields) when set; the stub ignores it until T7 implements the body.
fn run_doctor(json: bool, roundtrip: bool, compare_secs: Option<u64>) -> i32 {
    crate::doctor::run(roundtrip, compare_secs, json)
}

/// `observe`/`curve`/`hold`: write `cmd.json` atomically (R8). The daemon owns
/// all sysfs writes and re-validates/clamps; the CLI never touches sysfs.
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

/// `status`: merge config + state file + direct sysfs reads (R7), then print
/// Appendix B (`--json`) or a compact human summary. PHASE (b) verifies the merge.
fn run_status(json: bool, globals: &Globals) -> i32 {
    let (config, _warnings) = match Config::load(&globals.config) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let smc = match SysfsSmc::open(&globals.sysfs_root) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let sensors = match smc.read_sensors() {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let fan = match smc.read_fan() {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let (min_rpm, max_rpm) = (smc.hw_min_rpm(), smc.hw_max_rpm());
    let t_eff = sensors.iter().filter_map(|s| s.milli_c).max_by_key(|m| m.0);
    let state = read_state();
    let source = if globals.config.exists() {
        globals.config.display().to_string()
    } else {
        "defaults".to_string()
    };
    let temp = |m: MilliC| serde_json::json!(m.0 as f64 / 1000.0);
    let field = |key: &str| state.as_ref().and_then(|s| s.get(key));

    if json {
        let value = serde_json::json!({
            "schema": "afanctl.status.v1",
            "daemon": {"running": state.is_some(), "mode": field("mode").and_then(|m| m.as_str()).unwrap_or("observe"), "watchdog_armed": state.is_some(), "uptime_s": field("watchdog_pings").and_then(|p| p.as_u64()).unwrap_or(0)},
            "sensors": sensors.iter().map(|s| serde_json::json!({"label": s.label, "temp_c": s.milli_c.map_or(serde_json::Value::Null, temp)})).collect::<Vec<_>>(),
            "effective": {"temp_c": t_eff.map_or(serde_json::Value::Null, temp), "method": "max"},
            "fan": {"rpm": fan.rpm, "min_rpm": min_rpm, "max_rpm": max_rpm, "target_rpm": field("target_rpm").cloned().unwrap_or(serde_json::Value::Null), "manual": fan.mode == FanMode::Manual},
            "config": {"high_c": config.high_c, "max_c": config.max_c, "min_rpm": config.min_rpm, "max_rpm": config.max_rpm, "interval_s": config.interval_s, "source": source},
            "recent_errors": field("recent_errors").cloned().unwrap_or(serde_json::Value::Array(Vec::new())),
        });
        println!("{value}");
        return 0;
    }

    println!(
        "daemon: {}",
        if state.is_some() {
            "running"
        } else {
            "not running"
        }
    );
    for sensor in &sensors {
        let reading = sensor
            .milli_c
            .map_or_else(|| "n/a".to_string(), |m| format!("{:.1} C", temp_c(m)));
        println!("sensor {}: {reading}", sensor.label);
    }
    println!(
        "t_eff: {}",
        t_eff.map_or_else(|| "n/a".to_string(), |m| format!("{:.1} C", temp_c(m)))
    );
    println!(
        "fan: {} rpm ({}..{}, manual={})",
        fan.rpm,
        min_rpm,
        max_rpm,
        fan.mode == FanMode::Manual
    );
    println!(
        "config: high={} max={} min_rpm={} max_rpm={} interval_s={} source={source}",
        config.high_c, config.max_c, config.min_rpm, config.max_rpm, config.interval_s
    );
    0
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
                },
            ),
            (
                vec!["once", "--at-temp", "80", "--dry-run"],
                Command::Once {
                    at_temp: Some(80),
                    dry_run: true,
                },
            ),
            (
                vec!["once", "--at-temp", "-5"],
                Command::Once {
                    at_temp: Some(-5),
                    dry_run: false,
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
            vec!["once", "--json"],
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
}
