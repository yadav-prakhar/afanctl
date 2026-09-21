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
/// RULING F26 (2026-09-21): logical mode only. The *token* written and read back is resolved
/// from the bound applesmc ABI generation, never hardcoded: legacy spells Auto `0`, modern
/// spells it `2` and rejects `0` with `-EINVAL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode { Auto, Manual }
/// RULING F26: which applesmc attribute generation `open()` bound, detected by probing for the
/// mode attribute — never by parsing a kernel version. Legacy (<= 7.2): `fan1_output` +
/// `fan1_manual`, Auto = `0`. Modern (>= 7.3, commit 94f5081d): `fan1_target` + `pwm1_enable`,
/// Manual = `1`, Auto = `2`, `fan1_max` read-only, no `pwm1`. A tree exposing both mode
/// attributes is refused, not guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiGeneration { Legacy, Modern }
impl AbiGeneration { pub fn token(self) -> &'static str; }
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
    /// RULING F27 (2026-09-21): the L2 death-path descriptor — the pre-opened O_WRONLY fd on the
    /// attribute that returns control to the firmware, plus the exact `'static` bytes that do it.
    /// `None` when the backend has no such fd (read-only tree, or a mock without one). Presence is
    /// NOT proof; see `probe_safe_restore`. Replaces `panic_fd() -> Option<i32>`.
    fn safe_restore(&self) -> Option<SafeRestore>;
    /// RULING F27 arm-time probe: one real write of the restore bytes through the very fd the
    /// signal handler will use, then a verified read-back proving the hardware reports firmware
    /// control. `Err` means L2 must be reported **absent**, never armed — L2 ignores write errors
    /// by design, so an unproven path would advertise a layer that does nothing.
    fn probe_safe_restore(&mut self) -> Result<SafeRestore, SmcError>;
    /// RULING F27: safety layers beyond L1/L2 this backend can offer, for the status/state block.
    fn safety_capabilities(&self) -> SafetyCapabilities;
}
pub struct SysfsSmc { /* … */ }
impl SysfsSmc {
    pub fn open(root: &std::path::Path) -> Result<Self, SmcError>;
    pub fn layout_changed(&self) -> bool;                 // ruling T3 N2
    /// RULING F26: the bound generation, and its human evidence line for `doctor` (assembled from
    /// the one per-generation table, so R1 keeps attribute names inside `smc.rs`).
    pub fn abi_generation(&self) -> AbiGeneration;
    pub fn abi_evidence(&self) -> String;
}
pub struct MockSmc { /* … */ }   // scriptable: injectable faults, drift, latency
impl MockSmc {
    pub fn new(hw_min: u32, hw_max: u32) -> Self;         /* + fault injection setters */
    /// RULING F27: give the mock an L2 descriptor (a writable stand-in file + its `'static`
    /// bytes) so the descriptor and its arm-time probe are testable without hardware.
    pub fn set_safe_restore(&mut self, file: std::fs::File, bytes: &'static [u8]);
}

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
    /// RULING F27 (orchestrator, 2026-09-21): the order gains the arm-time probe — read the
    /// inherited fan mode → `probe_safe_restore()` → arm L2 only on success (else log `L2 death
    /// path ABSENT` and degrade to observe) → reconcile with the *pre-probe* snapshot → READY.
    /// The inherited mode must be sampled before the probe, because the probe's own fail-safe
    /// write would otherwise erase the evidence that a predecessor died owning the fan. R3 is
    /// read as "observe issues no *control* write": the probe writes only the fail-safe value.
    pub fn run(&mut self) -> !;
}
// RULING F18 (A4, N-F14-1): `config_source` is the resolved global `--config`
// path, populated by `cli.rs`, so the startup evidence line can log it.
pub struct RuntimePaths { pub cmd: std::path::PathBuf, pub state: std::path::PathBuf, pub config_source: std::path::PathBuf }

// ---- safety.rs — the ONLY module allowed `unsafe`; names NO attribute and NO restore value
/// RULING F27 (2026-09-21): everything the async-signal-safe death path needs, resolved up front
/// by the backend. `bytes` is `'static` — a per-backend constant slice, never heap or formatted —
/// so the handler never touches the allocator; a multi-byte restore ("level auto") is still one
/// write(2) and its length is part of the descriptor. A negative fd is the disarmed descriptor.
#[derive(Debug, Clone, Copy)]
pub struct SafeRestore { /* fd: RawFd, bytes: &'static [u8] */ }
impl SafeRestore {
    pub fn new(fd: std::os::fd::RawFd, bytes: &'static [u8]) -> Self;
    pub fn fd(&self) -> std::os::fd::RawFd;
    pub fn bytes(&self) -> &'static [u8];
}
/// RULING F27: safety layers a backend offers beyond L1/L2, for the status/state safety block.
/// `firmware_auto_on_suspend: None` = unproven for this backend; never asserted without evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyCapabilities {
    pub backend: &'static str,            // e.g. "applesmc/modern" — names no attribute (R1)
    pub hw_watchdog: bool,                // a firmware/EC watchdog that survives SIGKILL
    pub firmware_auto_on_suspend: Option<bool>,
}
/// Arm the death path with a backend-supplied descriptor + install the panic hook and raw
/// SIGSEGV/SIGABRT/SIGTERM/SIGINT handlers, which write its bytes to its fd in a single write(2).
/// Async-signal-safe only: ONE lock-free atomic load (the descriptor lives in leaked `'static`
/// storage published at arm time, so fd+ptr+len arrive together — no torn multi-atomic read),
/// one write(2), no allocation, no formatting, no locks, no path construction.
/// The caller must have proven the restore first (`Smc::probe_safe_restore`).
/// Replaces `install_death_path(panic_fd: i32)`.
pub fn install_death_path(restore: SafeRestore);
/// RULING F27: true when a *usable* descriptor is armed — the single source of truth for "L2
/// really will write on death", whoever armed it. One lock-free atomic load.
pub fn is_armed() -> bool;
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
  "safety": { "backend": "applesmc/modern", "l1_verify": true, "l2_death_path": true,
              "l3_watchdog_notify": true, "hw_watchdog": false,
              "firmware_auto_on_suspend": null },
  "recent_errors": [ { "ts": "2026-09-14T15:02:11+05:30", "msg": "…" } ]
}
```
RULING F27 (additive; schema id stays `v1`, the F18/F20/F22 precedent): `safety` reports which
layers are **actually armed**, per backend, instead of a single boolean the plugin had to infer.
`l2_death_path` comes from `safety::is_armed()`. `firmware_auto_on_suspend` is `boolean|null`,
`null` meaning unproven for this backend (applesmc today). In `status --json` the whole object is
`null` when no daemon state is published — `status` reads sysfs read-only and cannot observe
another process's armed layers, so absence of evidence is reported as such, never guessed at.
`afanctl.cmd.v1` (`/run/afanctl/cmd.json`, atomic tmp+rename, written by CLI verbs):
```json
{ "schema": "afanctl.cmd.v1", "mode": "hold", "rpm": 3000 }
```
`afanctl.state.v1` (`/run/afanctl/state.json`, rewritten by the daemon each poll):
```json
{ "schema": "afanctl.state.v1", "ts": "…", "mode": "curve", "t_eff_c": 71.2,
  "target_rpm": 3400, "last_written_rpm": 3350, "actual_rpm": 3390,
  "verified": true, "monitor_only": false, "auto_restore_pending": false, "polls": 981,
  "watchdog_pings": 1962,
  "safety": { "backend": "applesmc/modern", "l1_verify": true, "l2_death_path": true,
              "l3_watchdog_notify": true, "hw_watchdog": false,
              "firmware_auto_on_suspend": null },
  "recent_errors": [ … last 5 … ] }
```

### Appendix C — doctor output (human; `--json` reuses the same fields)

RULING F26: check **names** are a public contract and stay generation-neutral — the bound ABI
generation appears in the *details*, plus one additive check, `applesmc ABI generation`, whose
detail string is built inside `smc.rs` (R1). Checklist lines
`PASS|FAIL|WARN — <check> — <detail>` covering R5's list, then (if `--compare N`) a table: per-sample `t_eff`, SMC rpm, our simulated target, delta; summary stats (mean/max delta, samples where ours is quieter/louder); one-line verdict guidance. Exit 1 if any FAIL.

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

