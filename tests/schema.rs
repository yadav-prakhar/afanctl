#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Schema tests (PRD R10, owned by T8): the real binary's `status --json`
//! output is validated against the versioned `afanctl.status.v1` schema, and
//! the plugin-facing `afanctl.cmd.v1` / `afanctl.state.v1` files are shape-
//! checked (Appendix B). Actuating verbs run against a TEMPDIR fixture copy;
//! the repo fixture is never mutated.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn unique_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "afanctl-t8-schema-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_afanctl"))
}

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

/// Private writable fixture copy + runtime dir; removed on drop.
struct Sandbox {
    dir: PathBuf,
    root: PathBuf,
    run: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = unique_dir("box");
        let root = dir.join("root");
        let run = dir.join("run");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&run).expect("run");
        copy_tree(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs/devices"),
            &root.join("devices"),
        );
        Self { dir, root, run }
    }

    fn invoke(&self, args: &[&str]) -> Output {
        Command::new(binary())
            .env("AFANCTL_RUNTIME_DIR", &self.run)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("spawn afanctl")
    }

    fn root_str(&self) -> &str {
        self.root.to_str().expect("utf8 root")
    }

    fn read_json(&self, name: &str, expect_success: &Output) -> serde_json::Value {
        assert!(
            expect_success.status.success(),
            "verb failed: {:?}",
            expect_success
        );
        let path = self.run.join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is not JSON: {e}"))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn is_temp_or_null(v: &serde_json::Value) -> bool {
    v.is_f64() || v.is_null()
}

/// Assert a value matches the `afanctl.status.v1` schema (Appendix B).
fn assert_status_v1(v: &serde_json::Value) {
    assert_eq!(v["schema"], "afanctl.status.v1", "schema tag");
    let daemon = v["daemon"].as_object().expect("daemon object");
    for key in [
        "running",
        "mode",
        "monitor_only",
        "watchdog_armed",
        "uptime_s",
    ] {
        assert!(daemon.contains_key(key), "daemon.{key} missing");
    }
    assert!(daemon["running"].is_boolean());
    assert!(
        daemon["monitor_only"].is_boolean(),
        "F18 A1: additive latch flag"
    );
    assert!(daemon["watchdog_armed"].is_boolean());
    assert!(daemon["uptime_s"].is_u64());
    assert!(daemon["mode"].is_string());

    let sensors = v["sensors"].as_array().expect("sensors array");
    assert!(!sensors.is_empty(), "at least one sensor");
    for sensor in sensors {
        assert!(sensor["label"].is_string(), "sensor.label");
        assert!(
            is_temp_or_null(&sensor["temp_c"]),
            "sensor.temp_c numeric|null"
        );
    }

    assert_eq!(v["effective"]["method"], "max");
    assert!(is_temp_or_null(&v["effective"]["temp_c"]));

    let fan = &v["fan"];
    assert!(fan["rpm"].is_u64());
    assert!(fan["min_rpm"].is_u64());
    assert!(fan["max_rpm"].is_u64());
    assert!(fan["target_rpm"].is_u64() || fan["target_rpm"].is_null());
    assert!(fan["manual"].is_boolean());

    let config = &v["config"];
    for key in [
        "high_c",
        "max_c",
        "min_rpm",
        "max_rpm",
        "interval_s",
        "source",
    ] {
        assert!(config.get(key).is_some(), "config.{key} missing");
    }
    assert!(config["high_c"].is_i64());
    assert!(config["interval_s"].is_u64());
    assert!(config["source"].is_string());

    assert!(v["recent_errors"].is_array(), "recent_errors array");
}

/// Assert a value matches the `afanctl.cmd.v1` schema (Appendix B, R8).
fn assert_cmd_v1(v: &serde_json::Value) {
    assert_eq!(v["schema"], "afanctl.cmd.v1", "schema tag");
    let mode = v["mode"].as_str().expect("cmd.mode string");
    assert!(
        ["observe", "curve", "hold"].contains(&mode),
        "known mode: {mode}"
    );
    if mode == "hold" {
        assert!(v["rpm"].is_u64(), "hold cmd carries an integer rpm");
    }
}

/// Assert a value matches the `afanctl.state.v1` schema (Appendix B, R7).
fn assert_state_v1(v: &serde_json::Value) {
    assert_eq!(v["schema"], "afanctl.state.v1", "schema tag");
    for key in [
        "ts",
        "mode",
        "t_eff_c",
        "target_rpm",
        "last_written_rpm",
        "actual_rpm",
        "verified",
        "monitor_only",
        "watchdog_pings",
        "recent_errors",
    ] {
        assert!(v.get(key).is_some(), "state.{key} missing");
    }
    assert!(v["ts"].is_string());
    assert!(v["mode"].is_string());
    assert!(is_temp_or_null(&v["t_eff_c"]));
    assert!(v["verified"].is_boolean());
    assert!(
        v["monitor_only"].is_boolean(),
        "F18 A1: additive latch flag"
    );
    assert!(v["watchdog_pings"].is_u64());
    assert!(v["recent_errors"].is_array());
}

#[test]
fn status_json_matches_afanctl_status_v1() {
    let sandbox = Sandbox::new();
    let out = sandbox.invoke(&[
        "status",
        "--json",
        "--config",
        "/nonexistent/afanctl/afanctl.toml",
        "--sysfs-root",
        sandbox.root_str(),
    ]);
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("status --json stdout not JSON: {e}"));
    assert_status_v1(&value);
    // Daemon down (fresh runtime dir): running=false, live sysfs still read.
    assert_eq!(value["daemon"]["running"], false);
    assert_eq!(value["daemon"]["mode"], "observe");
    assert_eq!(
        value["daemon"]["monitor_only"], false,
        "F18 A1: a healthy fixture daemon is not latched"
    );
    assert_eq!(value["fan"]["rpm"], 1200);
    assert_eq!(value["config"]["source"], "defaults");
}

#[test]
fn hold_cmd_file_matches_afanctl_cmd_v1() {
    let sandbox = Sandbox::new();
    let out = sandbox.invoke(&["hold", "1300", "--sysfs-root", sandbox.root_str()]);
    let cmd = sandbox.read_json("cmd.json", &out);
    assert_cmd_v1(&cmd);
    assert_eq!(cmd["mode"], "hold");
    assert_eq!(
        cmd["rpm"], 1300,
        "unclamped in-band rpm is written verbatim"
    );
}

#[test]
fn observe_and_curve_cmd_files_match_afanctl_cmd_v1() {
    let sandbox = Sandbox::new();
    let observe = sandbox.invoke(&["observe"]);
    let cmd = sandbox.read_json("cmd.json", &observe);
    assert_cmd_v1(&cmd);
    assert_eq!(cmd["mode"], "observe");
    assert!(cmd.get("rpm").is_none(), "non-hold commands carry no rpm");

    let curve = sandbox.invoke(&["curve"]);
    let cmd = sandbox.read_json("cmd.json", &curve);
    assert_cmd_v1(&cmd);
    assert_eq!(cmd["mode"], "curve");
}

#[test]
fn once_state_file_matches_afanctl_state_v1() {
    let sandbox = Sandbox::new();
    let out = sandbox.invoke(&[
        "once",
        "--config",
        "/nonexistent/afanctl/afanctl.toml",
        "--sysfs-root",
        sandbox.root_str(),
    ]);
    let state = sandbox.read_json("state.json", &out);
    assert_state_v1(&state);
    assert_eq!(state["mode"], "curve");
    assert_eq!(state["target_rpm"], 1200);
    assert_eq!(state["last_written_rpm"], 1200);
    assert_eq!(state["verified"], true);
    assert_eq!(
        state["monitor_only"], false,
        "F18 A1: a healthy once-write is not latched"
    );
}
