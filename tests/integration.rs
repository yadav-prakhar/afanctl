//! Integration tests (PRD R10, owned by T8): spawn the REAL binary
//! (`env!("CARGO_BIN_EXE_afanctl")`, std::process::Command only — no
//! assert_cmd) and drive it against a TEMPDIR COPY of `tests/fixtures/sysfs/`.
//!
//! Safety invariants: every actuating verb gets `--sysfs-root` at a tempdir
//! copy (the repo fixture is NEVER mutated — one incident exists), and the
//! runtime dir is a per-test `AFANCTL_RUNTIME_DIR` tempdir, never `/run`.
//! The `hw` test only runs behind `--features hw` + `AFANCTL_HWTEST=1` +
//! real applesmc, and skips cleanly otherwise (PRD §8).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Per-process monotonic counter so parallel tests never share tempdirs.
fn unique_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("afanctl-t8-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Absolute path to the freshly built binary (cargo sets this for integration
/// tests of a binary target).
fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_afanctl"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Recursive copy of a directory tree using `cp -a` (std has no recursive
/// copy; `cp` is a system tool, not a crate dependency). `dst` must not exist.
fn copy_tree(src: &Path, dst: &Path) {
    let status = Command::new("cp")
        .arg("-a")
        .arg(src)
        .arg(dst)
        .status()
        .expect("run cp");
    assert!(
        status.success(),
        "cp -a {} {} failed",
        src.display(),
        dst.display()
    );
}

/// A private writable copy of the fixture's `devices/` tree. Dropped (removed)
/// at test end; the repo fixture is only ever read.
struct Fixture {
    dir: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = unique_dir("fix");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).expect("fixture root");
        copy_tree(
            &repo_root().join("tests/fixtures/sysfs/devices"),
            &root.join("devices"),
        );
        Self { dir, root }
    }

    fn fan(&self, name: &str) -> PathBuf {
        self.root.join("devices/platform/applesmc.768").join(name)
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.fan(name))
            .unwrap_or_else(|e| panic!("read {}: {e}", name))
            .trim()
            .to_string()
    }

    fn write(&self, name: &str, value: &str) {
        std::fs::write(self.fan(name), value).unwrap_or_else(|e| panic!("write {name}: {e}"));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The per-test runtime dir where `cmd.json` / `state.json` live.
struct RunDir {
    dir: PathBuf,
}

impl RunDir {
    fn new() -> Self {
        Self {
            dir: unique_dir("run"),
        }
    }
    fn cmd(&self) -> PathBuf {
        self.dir.join("cmd.json")
    }
    fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }
    fn log(&self) -> PathBuf {
        self.dir.join("daemon.log")
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run one shot of the real binary with `AFANCTL_RUNTIME_DIR` set, capturing
/// stdout/stderr. Never inherits the parent env's runtime dir.
fn run(run_dir: &RunDir, args: &[&str]) -> Output {
    Command::new(binary())
        .env("AFANCTL_RUNTIME_DIR", &run_dir.dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn afanctl")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Write a valid config with a chosen poll interval (defaults otherwise).
fn config(run_dir: &RunDir, interval_s: u64) -> PathBuf {
    let path = run_dir.dir.join("afanctl.toml");
    std::fs::write(
        &path,
        format!(
            "[thresholds]\nhigh = 66\nmax = 86\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n[poll]\ninterval_s = {interval_s}\n"
        ),
    )
    .expect("write config");
    path
}

/// Poll `pred` every 50 ms until true or `timeout` elapses.
fn wait_until(mut pred: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if pred() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The daemon's published mode, if `state.json` is readable.
fn state_mode(run_dir: &RunDir) -> Option<String> {
    let text = std::fs::read_to_string(run_dir.state()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("mode").and_then(|m| m.as_str()).map(str::to_owned)
}

/// The daemon's full published state, if readable.
fn state_json(run_dir: &RunDir) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(run_dir.state()).ok()?;
    serde_json::from_str(&text).ok()
}

/// RULING F18 (A1): the monitor-only/degraded latch from the published state.
fn state_monitor_only(run_dir: &RunDir) -> bool {
    state_json(run_dir)
        .and_then(|v| v.get("monitor_only").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// RULING F18 test 2: make `fan1_output` unwritable so the verified write
/// fails. `chmod 0444` is the requested shape; when the runner is root
/// (permission bits are ignored) fall back to a directory in the file's place,
/// so the write syscall fails with EISDIR regardless of uid.
fn make_unwritable(path: &Path) {
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(path, perms).expect("chmod 0444");
    if std::fs::write(path, b"0").is_ok() {
        std::fs::remove_file(path).expect("remove file");
        std::fs::create_dir(path).expect("replace with dir");
    }
}

/// A spawned `daemon` process. Killed (SIGKILL) on drop as a backup; use
/// [`Daemon::term_and_wait`] to exercise the L2 SIGTERM path deliberately.
struct Daemon {
    child: Child,
}

impl Daemon {
    fn spawn(run_dir: &RunDir, fixture: &Fixture, config: &Path, mode: &str) -> Self {
        let log = std::fs::File::create(run_dir.log()).expect("daemon log");
        let child = Command::new(binary())
            .env("AFANCTL_RUNTIME_DIR", &run_dir.dir)
            .args(["daemon", "--mode", mode, "--config"])
            .arg(config)
            .arg("--sysfs-root")
            .arg(&fixture.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .expect("spawn daemon");
        Self { child }
    }

    /// SIGTERM (not SIGKILL) so the L2 handler runs; returns true once the
    /// process has exited (a signal death has no exit code), false on timeout.
    fn term_and_wait(&mut self) -> bool {
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("kill -TERM {}", self.child.id()))
            .status();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => return false,
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `once`: one control iteration over the fixture copy prints the decision and
/// actuates through the verify path (curve at 45 °C → SetSpeed(1200)) — and
/// restores AUTO before exiting (F1: the verb must never leave `fan1_manual`
/// armed; the C1 end-state is impossible by normal operation).
#[test]
fn once_prints_decision_and_actuates_verify_path() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let out = run(
        &run_dir,
        &[
            "once",
            "--config",
            "/nonexistent/afanctl/afanctl.toml",
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(out.status.success(), "once must exit 0: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("mode=curve"), "{stdout}");
    assert!(stdout.contains("t_eff=45.0C"), "{stdout}");
    assert!(stdout.contains("decision=SetSpeed(1200)"), "{stdout}");
    assert!(stdout.contains("applied_rpm=1200"), "{stdout}");
    assert!(stdout.contains("verified=true"), "{stdout}");
    // The write-verify path committed (output holds the written rpm), then
    // the exit restore handed the fan back to the firmware (F1).
    assert_eq!(fixture.read("fan1_output"), "1200");
    assert_eq!(
        fixture.read("fan1_manual"),
        "0",
        "once must restore AUTO on a clean exit (F1)"
    );
    assert!(stdout.contains("exit: AUTO restored"), "{stdout}");
}

/// F1: `once` must also clear a pre-existing C1 state — a fixture left at
/// `fan1_manual=1` by an earlier crashed process comes back as AUTO after one
/// `once` iteration, and the restore failure path stays a nonzero exit.
#[test]
fn once_restores_auto_even_from_a_preexisting_manual_state() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    fixture.write("fan1_manual", "1");
    let out = run(
        &run_dir,
        &[
            "once",
            "--config",
            "/nonexistent/afanctl/afanctl.toml",
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(out.status.success(), "once must exit 0: {}", stderr(&out));
    assert_eq!(
        fixture.read("fan1_manual"),
        "0",
        "the C1 end-state must not survive a `once`"
    );
}

/// F2/D-T9-F2: `doctor` honors the `--sysfs-root` and `--config` globals —
/// it must report the FIXTURE's 45.0 C (never the live machine's temps), and
/// a present-but-invalid config is a FAIL naming the key (exit 1).
#[test]
fn doctor_honors_sysfs_root_and_config_globals() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let root = fixture.root.to_str().expect("utf8 root");

    let ok = run(
        &run_dir,
        &[
            "doctor",
            "--sysfs-root",
            root,
            "--config",
            "/nonexistent/afanctl/afanctl.toml",
        ],
    );
    assert!(
        ok.status.success(),
        "healthy fixture must pass: {}",
        stderr(&ok)
    );
    let stdout = String::from_utf8_lossy(&ok.stdout);
    assert!(
        stdout.contains("45.0"),
        "fixture t_eff expected (not the live machine): {stdout}"
    );

    let bad = run_dir.dir.join("bad.toml");
    std::fs::write(
        &bad,
        "[thresholds]\nhigh = 90\nmax = 80\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n[poll]\ninterval_s = 1\n",
    )
    .expect("write bad config");
    let out = run(
        &run_dir,
        &[
            "doctor",
            "--sysfs-root",
            root,
            "--config",
            bad.to_str().expect("utf8 bad.toml"),
        ],
    );
    assert!(!out.status.success(), "a bad config must FAIL doctor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("config validation"), "{stdout}");
    assert!(stdout.contains("thresholds.high"), "key named: {stdout}");
}

/// `hold` writes `cmd.json` only (never sysfs); a spawned `daemon` then applies
/// it within one poll — and SIGTERM exercises L2 (AUTO restored on the way out).
#[test]
fn hold_cmd_is_applied_by_daemon_within_one_poll() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    // One long poll: the daemon performs the first step immediately, then
    // sleeps, so the assertions cannot race a second decision.
    let cfg = config(&run_dir, 10); // legal long poll (< the 12 s watchdog cap)

    let hold = run(
        &run_dir,
        &[
            "hold",
            "1300",
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(hold.status.success(), "hold must exit 0: {}", stderr(&hold));
    assert_eq!(
        fixture.read("fan1_manual"),
        "0",
        "the CLI never touches sysfs"
    );

    let cmd: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.cmd()).expect("cmd.json"))
            .expect("json");
    assert_eq!(cmd["schema"], "afanctl.cmd.v1");
    assert_eq!(cmd["mode"], "hold");
    assert_eq!(cmd["rpm"], 1300);

    let mut daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "observe");
    let applied = wait_until(
        || state_mode(&run_dir).as_deref() == Some("hold"),
        Duration::from_secs(5),
    );
    if !applied {
        let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
        panic!("daemon never applied the queued hold; daemon log:\n{log}");
    }
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.state()).expect("state.json"))
            .expect("json");
    assert_eq!(state["mode"], "hold");
    assert_eq!(state["target_rpm"], 1300);
    assert_eq!(state["last_written_rpm"], 1300, "verified write committed");
    assert_eq!(fixture.read("fan1_manual"), "1");
    assert_eq!(fixture.read("fan1_output"), "1300");

    // L2 signal path: SIGTERM → single write(b"0") → AUTO.
    assert!(daemon.term_and_wait(), "daemon must exit on SIGTERM");
    assert_eq!(fixture.read("fan1_manual"), "0", "L2 restored AUTO");
}

/// `selftest-panic` exits nonzero AND the L2 panic hook writes AUTO to the
/// fixture copy (PRD R4/§9.3c). The fixture starts at `1` so the `0` is
/// evidence, not a coincidence.
#[test]
fn selftest_panic_exits_nonzero_and_restores_auto() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    fixture.write("fan1_manual", "1");
    let out = run(
        &run_dir,
        &[
            "selftest-panic",
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(!out.status.success(), "the deliberate panic exits nonzero");
    assert_eq!(
        fixture.read("fan1_manual"),
        "0",
        "the L2 panic hook wrote b\"0\" (AUTO)"
    );
}

/// L1 re-asserts manual mode after induced drift (R4), with no operator help.
#[test]
fn l1_reasserts_mode_drift() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);

    let hold = run(
        &run_dir,
        &[
            "hold",
            "1200",
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(hold.status.success(), "hold must exit 0: {}", stderr(&hold));

    let _daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "observe");
    let armed = wait_until(
        || fixture.read("fan1_manual") == "1" && state_mode(&run_dir).as_deref() == Some("hold"),
        Duration::from_secs(5),
    );
    if !armed {
        let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
        panic!("daemon never armed the manual hold; daemon log:\n{log}");
    }

    // Induce mode drift: the firmware/SMC flips the fan back to AUTO.
    fixture.write("fan1_manual", "0");
    let restored = wait_until(
        || fixture.read("fan1_manual") == "1",
        Duration::from_secs(5),
    );
    assert!(restored, "L1 must re-assert manual mode after drift");
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
    }
    // Real-hardware assertions are added only in the supervised gate (§9.3);
    // this test exists to prove the triple guard compiles and skips cleanly.
}

// ---- F14: startup reconcile (RULING F14) ----

/// F14: a fixture pre-set to `fan1_manual=1` (the SIGKILLed-predecessor state)
/// is reconciled back to AUTO by the observe daemon's startup — within one
/// poll `fan1_manual` reads `0` (R3: observe writes nothing after reconcile,
/// so the restore is the only write the process ever makes).
#[test]
fn daemon_observe_reconciles_stale_manual_state() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);
    fixture.write("fan1_manual", "1");

    let _daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "observe");
    let reconciled = wait_until(
        || fixture.read("fan1_manual") == "0",
        Duration::from_secs(5),
    );
    if !reconciled {
        let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
        panic!("observe daemon never reconciled fan1_manual=1 → 0; daemon log:\n{log}");
    }
    // Reconcile is idempotent: the healthy path writes nothing further.
    assert_eq!(
        fixture.read("fan1_output"),
        "1200",
        "observe writes no fan1_output (reconcile only restores the mode)"
    );
}

/// F14: the `--mode curve` variant reconciles FIRST (the startup restore is
/// logged before any control write), then takes control: `fan1_manual` ends
/// at `1` with a real verified speed write — the reconcile must not eat the
/// commanded mode. The `0` intermediate is a microseconds-wide window (the
/// first poll follows reconcile immediately), so reconcile is asserted via
/// the daemon's own journal/state evidence, not by racing the fixture file.
#[test]
fn daemon_curve_reconciles_then_takes_control() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);
    fixture.write("fan1_manual", "1");

    let mut daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "curve");
    // Control: first poll re-arms manual and writes the curve target (45 °C
    // → SetSpeed(1200); the fixture input already sits at 1200, so verify the
    // write via the state file's committed result, not an rpm delta).
    let controlled = wait_until(
        || {
            fixture.read("fan1_manual") == "1"
                && std::fs::read_to_string(run_dir.state())
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|v| {
                        v.get("last_written_rpm")
                            .and_then(|r| r.as_u64())
                            .map(|r| r == 1200)
                    })
                    .unwrap_or(false)
        },
        Duration::from_secs(5),
    );
    if !controlled {
        let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
        panic!("curve daemon never took control after reconcile; daemon log:\n{log}");
    }
    // Reconcile evidence: the startup restore ran (before control) and the
    // state file carries it for `status --json` / the plugin.
    let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
    assert!(
        log.contains("startup reconcile: AUTO restored"),
        "reconcile must restore AUTO before control; log:\n{log}"
    );
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.state()).expect("state.json"))
            .expect("json");
    assert_eq!(
        state["mode"], "curve",
        "commanded mode survives the reconcile"
    );
    assert!(
        state["recent_errors"]
            .as_array()
            .expect("recent_errors")
            .iter()
            .any(|e| e["msg"]
                .as_str()
                .is_some_and(|m| m.contains("stale manual mode restored to AUTO"))),
        "reconcile recorded for the plugin: {state}"
    );
    assert!(daemon.term_and_wait(), "daemon must exit on SIGTERM");
    assert_eq!(fixture.read("fan1_manual"), "0", "L2 restored AUTO on exit");
}

// ---- F18: degraded-state observability + human status (RULING F18) ----

/// F18 A1 (ticket test 1): `status --json` carries `daemon.monitor_only`, and
/// a healthy fixture daemon reports it false.
#[test]
fn status_json_reports_monitor_only_false_for_healthy_daemon() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);
    let _daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "observe");
    assert!(
        wait_until(|| state_json(&run_dir).is_some(), Duration::from_secs(5)),
        "daemon must publish state.json"
    );

    let out = run(
        &run_dir,
        &[
            "status",
            "--json",
            "--config",
            cfg.to_str().expect("utf8 cfg"),
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(out.status.success(), "status must exit 0: {}", stderr(&out));
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("status --json stdout is JSON");
    assert_eq!(value["daemon"]["running"], true);
    assert_eq!(
        value["daemon"]["monitor_only"], false,
        "a healthy daemon must not report the degraded latch"
    );
    assert!(value["daemon"]["monitor_only"].is_boolean());
}

/// F18 A1/A2 (ticket test 2, the F17b regression): making the fixture's
/// `fan1_output` unwritable for a spawned `daemon --mode curve` fails the
/// verified write `WRITE_FAIL_FALLBACK` times ⇒ the state file reports
/// `monitor_only: true` with the fallback cause in `recent_errors`, and human
/// `status` marks the degraded mode (the display that misled a reader).
#[test]
fn curve_write_failure_degrades_and_status_marks_monitor_only() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);
    make_unwritable(&fixture.fan("fan1_output"));

    let _daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "curve");
    let degraded = wait_until(|| state_monitor_only(&run_dir), Duration::from_secs(10));
    if !degraded {
        let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
        panic!("daemon never latched monitor-only after write failures; log:\n{log}");
    }
    let state = state_json(&run_dir).expect("state.json");
    assert_eq!(
        state["mode"], "curve",
        "commanded mode is kept while degraded (the F17b trap)"
    );
    assert_eq!(state["verified"], false);
    assert!(
        state["recent_errors"]
            .as_array()
            .expect("recent_errors")
            .iter()
            .any(|e| e["msg"]
                .as_str()
                .is_some_and(|m| m.contains("monitor-only degradation"))),
        "the cause must stay visible in state.json: {state}"
    );

    let out = run(
        &run_dir,
        &[
            "status",
            "--config",
            cfg.to_str().expect("utf8 cfg"),
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(out.status.success(), "status must exit 0: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("mode: curve (monitor-only)"),
        "human status must mark the degraded state: {stdout}"
    );
    assert!(stdout.contains("recent_errors: "), "{stdout}");
}

/// F18 A2 (ticket test 3): human `status` prints the commanded mode, the live
/// target rpm and a recent_errors line — fixture-backed.
#[test]
fn status_human_prints_mode_target_and_recent_errors() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);
    let _daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "curve");
    let controlled = wait_until(
        || {
            state_json(&run_dir)
                .and_then(|v| v.get("target_rpm").and_then(serde_json::Value::as_u64))
                == Some(1200)
        },
        Duration::from_secs(5),
    );
    if !controlled {
        let log = std::fs::read_to_string(run_dir.log()).unwrap_or_default();
        panic!("curve daemon never published a target; daemon log:\n{log}");
    }

    let out = run(
        &run_dir,
        &[
            "status",
            "--config",
            cfg.to_str().expect("utf8 cfg"),
            "--sysfs-root",
            fixture.root.to_str().expect("utf8 root"),
        ],
    );
    assert!(out.status.success(), "status must exit 0: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("mode: curve"), "{stdout}");
    assert!(stdout.contains("target: 1200 rpm"), "{stdout}");
    assert!(stdout.contains("recent_errors:"), "{stdout}");
}

// ---- F24: mode changes reach the daemon's journal (RULING F24, PRD R7) ----

/// RULING F24 (ticket test 1): the daemon's own captured log names every real
/// mode transition at INFO — the hardware-gate (f) evidence the old build threw
/// away (`run()` discarded the `StepReport`). Commands go through the CLI
/// verbs, which write `cmd.json`; the daemon re-reads and applies each poll.
#[test]
fn daemon_journal_names_mode_changes() {
    let run_dir = RunDir::new();
    let fixture = Fixture::new();
    let cfg = config(&run_dir, 1);
    let root = fixture.root.to_str().expect("utf8 root").to_owned();
    let cfg_s = cfg.to_str().expect("utf8 cfg").to_owned();
    let _daemon = Daemon::spawn(&run_dir, &fixture, &cfg, "observe");
    let journal = || std::fs::read_to_string(run_dir.log()).unwrap_or_default();
    assert!(
        wait_until(
            || journal().contains("daemon startup"),
            Duration::from_secs(5)
        ),
        "daemon must log its startup evidence line"
    );

    for (verb, args, expect) in [
        ("curve", vec!["curve"], "mode change: observe -> curve"),
        (
            "hold",
            vec!["hold", "3000"],
            "mode change: curve -> hold 3000",
        ),
        (
            "observe",
            vec!["observe"],
            "mode change: hold 3000 -> observe",
        ),
    ] {
        let mut argv = args.clone();
        argv.extend(["--config", cfg_s.as_str(), "--sysfs-root", root.as_str()]);
        let out = run(&run_dir, &argv);
        assert!(
            out.status.success(),
            "{verb} verb must exit 0: {}",
            stderr(&out)
        );
        assert!(
            wait_until(|| journal().contains(expect), Duration::from_secs(5)),
            "journal must name `{expect}` after `{verb}`; log:\n{}",
            journal()
        );
    }

    assert!(
        journal().contains("released to AUTO"),
        "the observe transition must name the release; log:\n{}",
        journal()
    );
    assert_eq!(state_mode(&run_dir).as_deref(), Some("observe"));
}
