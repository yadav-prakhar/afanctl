## 7. Architecture & interface contracts (BINDING — Appendix A)

Crate layout and per-file budget:

```
afanctl/
├── Cargo.toml                 (T0)
├── DESIGN.md                  (T0: copy of §7+§8 — in-repo single source of truth)
├── DEVIATIONS.md  QUESTIONS.md (T0: cross-agent coordination files)
├── src/
│   ├── main.rs       ~60   wiring only
│   ├── cli.rs        ~200  verb dispatch, arg parsing, output formatting
│   ├── config.rs     ~150  typed TOML, validation, provenance
│   ├── policy.rs     ~200  pure Controller state machine + constants
│   ├── smc.rs        ~230  sysfs adapter + MockSmc + read-back-verify
│   ├── supervisor.rs ~250  run modes, poll loop, L1, command/state files
│   ├── safety.rs     ~90   L2 death path (pre-opened fd, hooks) + selftest
│   ├── notify.rs     ~40   hand-rolled sd_notify
│   └── doctor.rs     ~200  diagnostics + curve comparison
├── tests/  (policy_traces.rs, integration.rs, schema.rs)
├── tests/fixtures/sysfs/     (fake applesmc + coretemp tree)
└── packaging/ (afanctl.service, afanctl.toml.default, PKGBUILD, polkit rule)
```

**Units:** temperatures are `MilliC(pub i32)` (milli-°C, exactly as sysfs provides) until display; rpm are bare `u32` (hw max 7200). No bare `i32` temps or stringly-typed values cross module boundaries.

**Error model:** one `thiserror` enum per module (`ConfigError`, `SmcError`, `SupError`). Library code returns `Result`; no `unwrap`/`expect` outside tests and `main.rs` (clippy-gated).

### Appendix A — exact public signatures

```rust
// ---- units.rs (may live inside policy.rs; T0 decides, then it is frozen)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilliC(pub i32);
impl MilliC { pub fn from_c(c: i32) -> Self; pub fn as_c(&self) -> i32 /* rounded */ }

// ---- smc.rs — the ONLY module that knows a sysfs path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode { Auto, Manual }
#[derive(Debug, Clone)]
pub struct FanState { pub rpm: u32, pub mode: FanMode }
#[derive(Debug, Clone)]
pub struct SensorReading { pub label: String, pub milli_c: Option<MilliC> } // None = failed/invalid read

#[derive(Debug, thiserror::Error)]
pub enum SmcError {
    #[error("not found: {0}")] NotFound(std::path::PathBuf),
    #[error("read {path}: {source}")] Read { path: std::path::PathBuf, source: std::io::Error },
    #[error("write {path}: {source}")] Write { path: std::path::PathBuf, source: std::io::Error },
    #[error("verify failed: wrote {wrote}, read back {read_back}")] VerifyFailed { wrote: u32, read_back: u32 },
    #[error("invalid value from {path}: {value}")] InvalidValue { path: std::path::PathBuf, value: String },
}

pub trait Smc: Send {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError>;
    fn read_fan(&self) -> Result<FanState, SmcError>;
    fn hw_min_rpm(&self) -> u32;
    fn hw_max_rpm(&self) -> u32;
    /// RULING F16 (orchestrator, 2026-09-14): write verification for `fan1_output` reads back
    /// **`fan1_output`** (the register we wrote), tolerance `WRITE_ECHO_TOLERANCE_RPM = 50`.
    /// `fan1_input` is the tachometer and must never be a write-verification source: a fan that
    /// is still spinning down is not a failed write. L1 splits mode-drift (a real failure,
    /// counted) from tracking (re-assert only, never counted); a genuine unresponsive actuator is
    /// caught by the `STALL_POLLS` stall detector. Reason: on real hardware the previous
    /// semantics disabled control during every ramp (see F16 evidence in the ledger).
    /// RULING F19 (orchestrator, 2026-09-14): the echo is verified inside a **settle window**
    /// (`ECHO_SETTLE_MS = 1500`, sampled 10 × 150 ms; `MODE_SETTLE_MS = 1000` for `set_mode`)
    /// because the SMC adopts `F0Tg` asynchronously on a ~1 s tick — measured on hardware:
    /// write 2000 ⇒ target reads 2000 within ≤1 s, full-swing convergence ~5 s. One write per
    /// window, no microsecond retries; `VerifyFailed` only when the window expires. `doctor`
    /// additionally WARNs when the running daemon predates the installed binary (stale-build trap).
    fn write_speed(&mut self, rpm: u32) -> Result<u32, SmcError>;
    /// Write + read-back-verify. Returns the verified mode.
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, SmcError>;
    /// Pre-opened O_WRONLY fd on the manual file for the L2 death path (None if backend is a mock without one).
    fn panic_fd(&self) -> Option<i32>;
}
pub struct SysfsSmc { /* … */ }
impl SysfsSmc { pub fn open(root: &std::path::Path) -> Result<Self, SmcError>; }
pub struct MockSmc { /* … */ }   // scriptable: injectable faults, drift, latency
impl MockSmc { pub fn new(hw_min: u32, hw_max: u32) -> Self; /* + fault injection setters */ }

// ---- config.rs
#[derive(Debug, Clone)]
pub struct Config { pub high_c: i32, pub max_c: i32, pub min_rpm: u32, pub max_rpm: u32, pub interval_s: u64 }
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("parse: {0}")] Parse(#[from] toml::de::Error),
    #[error("io {path}: {source}")] Io { path: std::path::PathBuf, source: std::io::Error },
    #[error("invalid {key}: {reason} (fix: {fix})")] Invalid { key: &'static str, reason: String, fix: String },
}
impl Config {
    pub fn from_toml(s: &str) -> Result<(Self, Vec<String> /* unknown-key warnings */), ConfigError>;
    pub fn load(path: &std::path::Path) -> Result<(Self, Vec<String>), ConfigError>; // missing file => defaults
    pub fn validate(&self, hw: (u32, u32)) -> Result<(), ConfigError>; // hw = (fan1_min, fan1_max)
    pub fn defaults() -> Self;
}
/// Config + derived values, fully validated against hardware. What Controller consumes.
#[derive(Debug, Clone)]
pub struct ResolvedConfig { pub config: Config, pub low_c: i32 /* = high-3 */ }

// ---- policy.rs — pure; no I/O, no clock, no threads
pub const SLEW_MAX_RPM_PER_POLL: u32 = 750;
pub const SENSOR_LOSS_POLLS: u32 = 3;
pub const OVERSHOOT_POLLS: u32 = 3;
pub const VERIFY_TOLERANCE_RPM: u32 = 150;   // consumed by supervisor L1
pub const WRITE_FAIL_FALLBACK: u32 = 3;
pub const WRITE_ECHO_TOLERANCE_RPM: u32 = 50; // RULING F16: fan1_output register echo
pub const STALL_POLLS: u32 = 10;              // RULING F16/F20: unresponsive-actuator window
pub const ECHO_SETTLE_MS: u64 = 1500;         // RULING F19: SMC adopts F0Tg on a ~1s tick
pub const ECHO_SETTLE_SAMPLES: u32 = 10;      // RULING F19: reads inside the settle window
pub const WRITE_RETRY_MAX: u32 = 1;           // RULING F19: single re-issue after a dead window
pub const MODE_SETTLE_MS: u64 = 1000;         // RULING F19: same, for the FS! manual bit
pub const STALL_TACH_EPSILON_RPM: u32 = 50;   // RULING F20: "the tach is not moving at all"
pub const AUTO_RETRY_LOG_POLLS: u32 = 10;     // RULING F20: rate-limit the failed-AUTO log
pub const OFF_TARGET_WARN_POLLS: u32 = 30;    // RULING F20: visibility for a slow/dead-ish fan

#[derive(Debug, PartialEq)]
pub enum Decision { Observe, SetSpeed(u32), EscalateMax, ReturnToAuto }

pub struct Controller { /* curve hysteresis state, slew state, streak counters */ }
impl Controller {
    pub fn new(cfg: &ResolvedConfig) -> Self;
    /// Curve mode step. Interim sensor loss (streak < SENSOR_LOSS_POLLS): keep the current
    /// output, write nothing new. Full streak → ReturnToAuto. Never holds a stale speed
    /// on valid input: target is recomputed from t_eff every call (no direction gates).
    pub fn step_curve(&mut self, readings: &[SensorReading]) -> Decision;
    /// Hold mode step: SetSpeed(held) normally, EscalateMax when the overshoot guard fires.
    pub fn step_hold(&mut self, readings: &[SensorReading], held_rpm: u32) -> Decision;
    /// Observe mode step: always Observe, but tracks t_eff + sensor-loss streak (for logging/status).
    pub fn step_observe(&mut self, readings: &[SensorReading]) -> Decision;
    /// Last effective temperature (max over valid sensors), for status.
    pub fn t_eff(&self) -> Option<MilliC>;
}

// ---- supervisor.rs
#[derive(Debug, Clone, PartialEq)]
pub enum RunMode { Observe, Curve, Hold(u32) }

#[derive(Debug)]
pub struct StepReport {
    pub mode: RunMode,
    pub t_eff: Option<MilliC>,
    pub decision: Decision,
    pub applied_rpm: Option<u32>,   // verified write result, None if no write
    pub verified: bool,             // L1 check outcome this poll
    pub notes: Vec<String>,         // human-readable events (re-asserts, fallbacks, cmd applied)
}

pub struct Supervisor { /* smc, controller, mode, l1 counters, cmd/state file paths */ }
impl Supervisor {
    pub fn new(smc: Box<dyn Smc>, cfg: &ResolvedConfig, start: RunMode,
               paths: &RuntimePaths) -> Result<Self, SupError>;
    /// Exactly one poll iteration: read cmd file → read sensors → decide → act+verify →
    /// L1 re-assert → write state.json. The watchdog is pinged at the **start and end**
    /// of the poll (RULING F21 R2: a long poll cannot extend the ping gap beyond
    /// `max(interval, poll_work)`). Returns the report (testable).
    pub fn step_once(&mut self) -> StepReport;
    /// Foreground loop (systemd Type=notify). Installs L2, notifies READY, runs forever.
    /// RULING F14 (orchestrator, 2026-09-14): startup order is now
    /// arm L2 → **reconcile stale state** → sd_status/sd_ready → poll loop.
    /// Reconcile: read the fan; if `Manual`, restore AUTO on the verified write path and log
    /// loudly; if that restore fails (or the fan cannot be read while a writing mode is
    /// commanded), degrade to `RunMode::Observe` + `monitor_only` — never command manual
    /// mode while AUTO cannot be restored. Idempotent: no write when the fan already reads
    /// `Auto`. Rationale: a SIGKILL/OOM-kill cannot run L2, so the *restart* is what restores
    /// AUTO within ~1 s; PRD §9.3e's "L2 already restored AUTO" names a mechanism that is
    /// impossible for an uncatchable signal, while its property remains the acceptance bar.
    pub fn run(&mut self) -> !;
}
// RULING F18 (A4, N-F14-1): `config_source` is the resolved global `--config`
// path, populated by `cli.rs`, so the startup evidence line can log it.
pub struct RuntimePaths { pub cmd: std::path::PathBuf, pub state: std::path::PathBuf, pub config_source: std::path::PathBuf }

// ---- safety.rs — the ONLY module allowed `unsafe`
/// Pre-open fd + install panic hook and raw SIGSEGV/SIGABRT/SIGTERM/SIGINT handlers that
/// write b"0" (AUTO) to the fan manual file in a single write(2). Async-signal-safe only.
pub fn install_death_path(panic_fd: i32);
/// Deliberate panic for `selftest-panic` (proves L2; verified by fixture/hw tests).
pub fn arm_test_panic() -> !;

// ---- doctor.rs (diagnostics; read-only unless --roundtrip)
//   Takes the CLI globals' root+config path explicitly — R5 globals are global
//   redirects; T9-F2: doctor silently ignoring them is a defect.
pub fn run(sysfs_root: &std::path::Path, config_path: Option<&std::path::Path>, roundtrip: bool, compare_secs: Option<u64>, json: bool) -> i32;
//   json=true renders Appendix C output as JSON (same fields; R5 verb table).

// ---- notify.rs (hand-rolled; no systemd crate)
pub fn sd_ready() -> bool;             // READY=1
pub fn sd_watchdog() -> bool;          // WATCHDOG=1
pub fn sd_status(msg: &str) -> bool;   // STATUS=…
```

### Appendix B — file & JSON schemas (versioned; breaking changes bump the `v1`)

`afanctl.status.v1` (`status --json`):
```json
{
  "schema": "afanctl.status.v1",
  "daemon": { "running": true, "mode": "curve", "monitor_only": false, "auto_restore_pending": false, "watchdog_armed": true, "uptime_s": 981 },
  "sensors": [ { "label": "Core 0", "temp_c": 53.0 } ],
  "effective": { "temp_c": 53.0, "method": "max" },
  "fan": { "rpm": 1203, "min_rpm": 1200, "max_rpm": 7200, "target_rpm": 1200, "manual": false },
  "config": { "high_c": 66, "max_c": 86, "min_rpm": 1200, "max_rpm": 6200, "interval_s": 1,
              "source": "/etc/afanctl/afanctl.toml" },
  "recent_errors": [ { "ts": "2026-09-14T15:02:11+05:30", "msg": "…" } ]
}
```
`afanctl.cmd.v1` (`/run/afanctl/cmd.json`, atomic tmp+rename, written by CLI verbs):
```json
{ "schema": "afanctl.cmd.v1", "mode": "hold", "rpm": 3000 }
```
`afanctl.state.v1` (`/run/afanctl/state.json`, rewritten by the daemon each poll):
```json
{ "schema": "afanctl.state.v1", "ts": "…", "mode": "curve", "t_eff_c": 71.2,
  "target_rpm": 3400, "last_written_rpm": 3350, "actual_rpm": 3390,
  "verified": true, "monitor_only": false, "auto_restore_pending": false, "polls": 981,
  "watchdog_pings": 1962, "recent_errors": [ … last 5 … ] }
```

### Appendix C — doctor output (human; `--json` reuses the same fields)

Checklist lines `PASS|FAIL|WARN — <check> — <detail>` covering R5's list, then (if `--compare N`) a table: per-sample `t_eff`, SMC rpm, our simulated target, delta; summary stats (mean/max delta, samples where ours is quieter/louder); one-line verdict guidance. Exit 1 if any FAIL.

### Appendix D — `packaging/afanctl.service`

```ini
[Unit]
Description=afanctl - applesmc fan supervisor (pre-T2 Intel Mac)
After=multi-user.target
StartLimitIntervalSec=0

[Service]
Type=notify
ExecStart=/usr/bin/afanctl daemon --mode observe
WatchdogSec=15
Restart=always
RestartSec=1
ProtectSystem=strict
ReadWritePaths=/sys/devices/platform/applesmc.768
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes
RuntimeDirectory=afanctl

[Install]
WantedBy=multi-user.target
```
Note: `ReadWritePaths` pins today's platform path; `doctor` must detect a layout change (hwmon conversion) and tell the user to update the unit — this coupling is deliberate and documented.

## 8. Engineering conventions (BINDING — paste verbatim into every subagent instruction)

**Language & toolchain**
- Rust edition 2021; rustc 1.98 (Arch `extra/rust`; install via `sudo pacman -S rust` — or `pkexec --disable-internal-agent pacman -S rust` on this box).
- No async. No threads beyond the main loop (single-threaded design; `Send` bound on `Smc` is for the trait's future, not for spawning).
- Dependencies: allowlist in R11 only. Adding one = stop, record in `DEVIATIONS.md`, await planner.

**Correctness discipline**
- No `unwrap`/`expect`/`panic!` outside tests, `main.rs` wiring, and `safety.rs`'s deliberate test panic. Clippy denies them (`clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic` — allowlisted per-file where required).
- `unsafe` only in `safety.rs`, each block carrying a `// SAFETY:` comment naming the async-signal-safety argument.
- Every state-changing `smc` write goes through write-verify (R1). Never update logical state from an unverified write (the `mbpfan.c:410` class).
- Temps are `MilliC`, never bare `i32` across module boundaries. rpm are `u32`.

**Style**
- `cargo fmt` default config, no customization. `//!` module doc on every file stating its contract and invariants in ≤10 lines. Public items get doc comments. Safety-critical lines get `// SAFETY:` or `// INVARIANT:` comments.
- Errors: per-module `thiserror` enum; error messages name the path/key and the fix. Logging via `tracing` macros only — no `println!` outside `cli.rs` output formatting.
- Tests: unit tests co-located (`#[cfg(test)]`); cross-module tests in `tests/`. Every behavioral claim in this PRD that is testable on fixtures/mocks **must** have a test (see R10 list).

**Safety of the build process itself (non-negotiable)**
- **Never write to the real `/sys` during development.** All tests run against `tests/fixtures/sysfs/` via `--sysfs-root`. Real-hardware tests run only behind `--features hw` + `AFANCTL_HWTEST=1` + applesmc present, and only in the final supervised gate (§9). If a test would write real sysfs without those guards, that test is a bug.
- Never run the daemon against real hardware as part of ordinary development; the supervised gate does that, with the user present.

**Cross-agent protocol**
- You may only create/modify the files listed in your task card. Everything else is read-only.
- Signatures come from Appendix A (repo copy: `DESIGN.md`). If a required change is discovered: implement everything else, record the proposed change in `DEVIATIONS.md` (old → new → why → which tasks are affected), and flag it in your final summary. Never silently rename/add public items.
- Ambiguity or a blocking question → write it to `QUESTIONS.md`, continue with the unambiguous remainder, and say so in your summary. Do not invent spec.
- LOC budget per file in §7; >20% over = stop and report (scope smell), do not absorb silently.

**Quality gates (every task, before declaring done)**
```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   # must SKIP cleanly (no applesmc in dev containers) — proves the guard works
```
Plus the task card's own checks. A task is done only when all gates pass in a clean checkout of its branch.

