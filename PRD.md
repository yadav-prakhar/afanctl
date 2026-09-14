# PRD — `afanctl`: an applesmc fan supervisor for the pre-T2 Intel Mac (A1708)

**Version:** 1.0 · **Date:** 2026-09-14 · **Status:** Ready for orchestration planning
**Owner:** prakhar · **Target machine:** MacBook Pro 2017 A1708 (MacBookPro14,1, pre-T2), Omarchy (Arch Linux), kernel 7.2.3-arch1-3, systemd PID1

> **For the planning agent:** this document is self-contained. You have no access to the conversations that produced it; everything you need is here or in the referenced source files. Your job is to translate §11 (Orchestration Brief) into a concrete plan of subagent tasks. §7 (Interface Contracts) and §8 (Engineering Conventions) are **binding** on every subagent — they are what make parallel work by multiple models converge into one coherent codebase.

---

## 1. Problem statement

The A1708's fan is driven by Apple's SMC firmware curve (via the `applesmc` kernel driver). The only maintained userspace controller for pre-T2 Macs is `mbpfan` — abandoned since 2019, and a two-agent adversarial review (critic × architect, four rounds, all claims re-verified against source and live hardware) concluded it is unfit to build on:

- **C1 (critical):** any crash/hang/start-limit-exhaustion leaves fans in manual mode at the last written speed with the SMC safety net disabled — silently, forever.
- **C2 (critical):** a failed sysfs read corrupts stack memory (`buf[-1]` underwrite and `buf[16]` overflow, `mbpfan.c:388-391`) in a root daemon, every poll.
- **H1:** it averages sensors, so one core at 92 °C hides behind an 80 °C average.
- **H2:** its direction-gated ratchet freezes fan speed on any temperature plateau — after a cooldown it locks in a permanent ~2.5× **overspeed** (≈3,560 rpm held vs ≈1,350 rpm target at 69 °C).
- **H3:** `high_temp == max_temp` passes its validation and produces UB in the speed calculation.
- Root cause, agreed by both agents: *nothing owns state, nothing observes outcomes* — reads, writes, cleanup, and death are all unverified.

The standard Linux alternative (`fancontrol`/`pwmconfig`) is structurally incompatible: it requires `pwm*` sysfs attributes that `applesmc` does not expose (verified live on this kernel). **There is no adoptable competitor.** The verdict: build a small, scoped successor — and make every failure path fail *toward* the firmware it replaces.

That successor is this project. It also has a second customer: the planned **omafan** Omarchy bar plugin (interactive fan control: auto preset, off/low/med/high/full presets, slider — see `omafan/prompt.md` referenced below). The plugin will front `afanctl`; it must not write sysfs itself.

## 2. Evidence base (all verified live on the target machine, 2026-09-14)

### 2.1 Hardware / system facts

| Fact | Value | Consequence |
|---|---|---|
| Fan count | 1 (`fan1_label` = "Exhaust") | Single-fan scope; keep a seam for more, don't build for more |
| `fan1_min` / `fan1_max` | **1200 / 7200 rpm** | Hardware range; all writes clamped here. (mbpfan's 6500 "max" was its own constant, not hardware) |
| Idle RPM (SMC auto) | ~1200 | SMC curve already idles near-silent — this is the incumbent to beat |
| `fan1_manual` | 0 = auto, 1 = manual | The safety-critical bit; `0` restores the firmware net |
| `fan1_output` | write target rpm | Actuator |
| `fan1_input` | actual rpm | Verification readback |
| `fan1_safe` | present, reads empty | Ignore; semantics unknown (open question Q3) |
| Sensors | `coretemp` → hwmon4 (dynamic index): "Package id 0", "Core 0", "Core 1" | 3 sensors; discovery must walk, never hardcode hwmonN |
| Tjmax (`temp*_crit`) | 100 °C | Config guard: user `max` ≤ 95 °C |
| applesmc hwmon dir | `hwmon3` exists but is an **attribute-less husk** | Upstream applesmc is mid-conversion to standard hwmon attrs; all sysfs knowledge must live in one module |
| applesmc also exposes | 33 raw SMC temp keys (`temp1..33`, labels like `TA0V`, `TB0T`) | Out of scope for control; coretemp is the trusted set |
| systemd | PID1, present | L3 supervision available |
| Rust | **not installed**; `extra/rust 1:1.98.1` available via pacman | Setup task required |
| mbpfan / t2fanrd | not installed | Clean slate; SMC firmware curve is currently in charge |

Sysfs root: `/sys/devices/platform/applesmc.768/` (fan files directly there, **not** under hwmon).

### 2.2 Naming collision (verified 2026-09-14)

`fanctl` is **taken**: a Rust crate and AUR package (`mcoffin/fanctl`, a fancontrol replacement, GPL3). `macfanctl` and `smctl` are taken (macOS tools). **This project's name is `afanctl`** (binary, crate, and AUR package; searched free today — re-verify on AUR + crates.io immediately before first publish, see Q1).

### 2.3 Source documents (optional deep reading; absolute paths)

- `/home/prakhar/Documents/Default/Workspace/omarchy plugin development/mbpfan-architecture-review/` — the four-round debate: `01-critique.md` (defect list w/ file:line evidence), `02-architect-response.md` (successor design + build-vs-adopt), `03-critique-rebuttal-response.md`, `04-architect-final-word.md` (**final rulings — the design authority this PRD implements**), `00-README.md` (summary).
- `/home/prakhar/Documents/Default/Workspace/omarchy plugin development/omafan/prompt.md` — the downstream plugin's requirements (presets, slider, publishing to plugins.omarchy.org).

## 3. Users & use cases

1. **prakhar (primary, direct):** wants quiet, correct, *observable* fan behavior on the A1708, and proof of what the controller is doing. Default interaction: `afanctl status` / `afanctl doctor`.
2. **omafan plugin (primary, programmatic):** an Omarchy/Quickshell bar widget that needs — auto preset (SMC default), off/low/med/high/full presets, and a slider. It consumes `afanctl`'s CLI verbs and `status --json` (Appendix B). It never touches sysfs.
3. **Other pre-T2 Intel Mac Linux users (secondary):** anyone with `applesmc` + `coretemp` and no `pwm` fan attrs. Supported incidentally, tested only on A1708.

**Use cases (ordered):** UC1 install & observe safely (default does nothing to the fan); UC2 diagnose ("is something controlling my fan? is my config valid?"); UC3 empirically compare SMC curve vs ours before committing (`doctor --compare`); UC4 run continuous curve control with every failure landing on the firmware; UC5 plugin/user sets a held speed or preset, with thermal guards; UC6 one-shot control decision for scripts/CI (`once`).

## 4. Goals & non-goals

**Goals**

1. A single small Rust daemon + CLI that replaces the *need* for mbpfan on this machine, with none of its defect classes (C1/C2/H1/H2/H3 impossible by construction).
2. **Fail-toward-AUTO as the universal failure policy**: every failure path — write failure, sensor loss, crash, hang, SIGKILL, start-limit storm, config error — ends with the SMC firmware back in charge, loudly logged. (Uncatchable deaths — SIGKILL/OOM-kill — cannot run L2; the startup reconcile restores AUTO on the restart, R4.)
3. Observability as a first-class feature: `status`, `status --json`, `doctor`, per-poll verification, structured runtime state file.
4. A stable, minimal programmatic surface the omafan plugin can drive without root-adjacent hacks of its own.
5. Buildable/testable **without touching real hardware** (fixture sysfs trees + mock backend); real-hw validation is a final, explicit, supervised gate.

**Non-goals (v1)**

- No T2 Macs, no Apple Silicon, no macOS, no multi-fan control (single-fan logic; seam only).
- No GUI — the plugin is a separate project.
- No in-process config reload — `systemctl restart` is the reload (H4 deleted by scope).
- No `pwm*` support, no below-`fan1_min` speeds (true fan-off is not reachable via applesmc sysfs — Q2).
- No `auto-intervene` mode in v1 (observe/curve/hold cover the plugin's needs; recorded as future work).
- No config sprawl: exactly 5 user-tunable values (§6.R6). Safety tunables are named constants, not config.

## 5. Priorities

- **P0 — safety core:** must ship as a unit or not at all.
- **P1 — plugin surface:** required before omafan can be built on this; in-scope for the first release.
- **P2 — polish/future:** explicitly deferred; do not build in v1 even if convenient.

## 6. Requirements

### R1 — Hardware interface (P0)

- One module (`smc`) owns **all** sysfs knowledge: discovery of fan files and coretemp sensors (walk `/sys/devices/platform/coretemp.0/hwmon/hwmon*/temp*_input` + `*_label`; the applesmc platform dir for `fan1_*`), path construction, reads, writes, retries. Nothing outside it contains a path string.
- **Read-back-verify is a module invariant, not a feature:** every state-changing write (`fan1_output`, `fan1_manual`) is followed by a read-back **of the attribute just written**, within tolerance; logical state commits only on verified read-back; failed verification retries up to `K=3`, then surfaces an error upward (which triggers R4 fail-toward-AUTO). `fan1_input` is the tachometer and is **never** a write-verification source — a fan that is still spinning down is not a failed write (ruling F16, DESIGN.md). This kills mbpfan's silent-write-failure class (H6/M8) by construction.
- All rpm writes clamped to [`fan1_min`, `fan1_max`] read from hardware at discovery.
- Sensor readings < 0 °C or > 120 °C are rejected as failed reads (outlier rejection).
- `--sysfs-root <dir>` dev/test flag redirects *all* sysfs access (how the entire test suite and the plugin-facing integration tests run against fixtures).

### R2 — Control policy (P0, pure & table-testable)

Implemented in `policy.rs` — no I/O, no clock, no threads. One `Controller` state machine:

- **Effective temperature** = max over valid sensors (acts on the 92 °C core, not the 80 °C average — kills H1). All sensors failing for `SENSOR_LOSS_POLLS = 3` consecutive polls → `ReturnToAuto`.
- **Absolute target curve, recomputed every poll, no direction gates** (kills H2 by construction):
  - `t < low` → `min_rpm`  (where `low = high − 3`, derived)
  - `low ≤ t < high` → `min_rpm`, with hysteresis: once ramping, stay ramping until `t < low` (no flapping)
  - `high ≤ t < max` → linear from `min_rpm` to `max_rpm`
  - `t ≥ max` → `max_rpm`
- **Slew limiter:** fan moves at most `SLEW_MAX_RPM_PER_POLL = 750` rpm per poll toward target (bounded transitions; full sweep ≈ 5 s at 1 Hz).
- **Overshoot guard:** `t_eff ≥ max − 1` for `OVERSHOOT_POLLS = 3` consecutive polls → `max_rpm` immediately, bypassing slew. **Applies in every mode, including hold.**
- **Hold mode safety:** hold clamps to hardware range; the overshoot guard overrides it; hold never disables L1/L2.

### R3 — Run modes (P0/P1)

| Mode | Writes? | Meaning | Entry |
|---|---|---|---|
| `observe` (**always the startup default**) | never writes `fan1_manual`/`fan1_output` | watches, logs, serves status/doctor; SMC firmware owns the fan | default on `daemon` start |
| `curve` | yes, continuous control | R2 loop drives the fan | `afanctl curve` verb; or unit override for mbpfan-like always-on |
| `hold(rpm)` (P1) | yes, fixed target | fan held at rpm (plugin presets/slider), with R2 guards | `afanctl hold <rpm>` verb |

- Mode changes at runtime flow through a **command file** (R8) re-validated and applied by the daemon each poll — the daemon owns every write; the CLI never writes sysfs.
- Startup mode is always `observe`, overridable to `curve` only via the systemd unit's `ExecStart` argument (opt-in, post-`doctor` evidence). `/run` is tmpfs: after reboot the machine is always back on the SMC curve.

### R4 — Safety layers (P0; the heart of the project)

- **L1 — per-poll verify/re-assert:** every poll re-reads `fan1_manual` and `fan1_input`. Two checks with different consequences (ruling F16, DESIGN.md): **(a) mode drift** — the fan reads `Auto` while we believe we own it ⇒ re-assert `set_mode(Manual)`; only this, write-syscall errors and register-echo failures count toward `WRITE_FAIL_FALLBACK = 3` ⇒ restore AUTO, degrade to monitor-only, log loudly. **(b) tracking** — while armed and the command is unchanged, `|actual − last_written| > VERIFY_TOLERANCE_RPM = 150` ⇒ re-issue the write, but **never count it as a failure**: a fan that is still decelerating is not a broken fan. A genuinely unresponsive actuator is caught by the **stall detector**: command unchanged for `STALL_POLLS = 10` polls and the deviation has not shrunk ⇒ failure ⇒ AUTO + monitor-only.
- **L2 — death path:** at startup pre-open an `O_WRONLY` fd on `fan1_manual`; a Rust panic hook plus raw `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers perform exactly one async-signal-safe `write(fd, b"0")` — restoring AUTO and the firmware net in a single syscall. No allocation, no formatting, no locks, no path construction. A hidden `afanctl selftest-panic` verb panics deliberately; tests (fixture) and the supervised hardware gate verify AUTO afterwards. **If L2 cannot be made trustworthy, the answer is observe mode — the tool never needed to flip the mode.**
- **L3 — systemd-first:** `Type=notify`, `WatchdogSec=15` (watchdog ping each poll; a hang gets SIGABRT → L2 → restart), `Restart=always`, `RestartSec=1`, `StartLimitIntervalSec=0` (crash-loops restart forever rather than exhausting into a dead fans-manual state — the corrected C1 mechanism), plus sandboxing (`ProtectSystem=strict`, `ReadWritePaths=/sys/devices/platform/applesmc.768`, `ProtectHome=yes`, `NoNewPrivileges=yes`, `PrivateTmp=yes`, `RuntimeDirectory=afanctl`). Full unit in Appendix D.
- **Sensor loss → AUTO** (R2) — fail toward the firmware, never drive on garbage.
- **Config invalid → refuse to start** (exit nonzero, loud error naming the key and the fix) — never run on defaults after a bad edit.

### R5 — CLI verbs (P0/P1)

Binary `afanctl`. Every verb: nonzero exit on failure; unknown flag → usage on stderr + exit 2 (fixing mbpfan's exit-0 disease). `--config <path>` and `--sysfs-root <path>` global.

| Verb | Pri | Behavior |
|---|---|---|
| `daemon` | P0 | run the supervisor loop (systemd Type=notify). `--mode observe\|curve` (default observe) |
| `status` | P0 | per-sensor temps, `t_eff`, mode, fan actual/target/min/max, manual?, config with provenance (default vs file), recent errors. Human output + `--json` (schema `afanctl.status.v1`, Appendix B) |
| `doctor` | P0 | diagnostic suite (below); `--roundtrip` opts into a 2-second manual-mode write test (the only write doctor ever does); `--compare <seconds>` runs the empirical curve comparison |
| `once` | P0 | one control iteration, print the decision, exit (scripts/CI). `--at-temp <C>` overrides sensor input (simulation); `--dry-run` prints without writing |
| `observe` / `curve` / `hold <rpm>` | P1 | write the command file (R8); `hold` clamps to hw range and rejects below-`fan1_min` with a clear message (see Q2) |
| `selftest-panic` | P0 | hidden; deliberate panic to prove L2 (fixture + supervised hw gate) |
| `--version`, `-h` | P0 | — |

`doctor` checks (all read-only unless `--roundtrip`): applesmc + coretemp present; fan files present & writable-by-root; sensor plausibility vs Tjmax; `fan1_min/max` readback; config validation; systemd unit health (notify/watchdog/start-limit configured? via `systemctl show`); applesmc layout-change detection (hwmon husk gaining fan attrs → warn "smc adapter needs updating"); L2 fd armed. `--compare N` (the debate's empirical gate): sample `t_eff` + SMC's own rpm for N seconds in observe mode, simulate our curve over the same trace, print a paired table + divergence stats — the user decides with data whether curve mode earns its keep.

### R6 — Configuration (P0)

Typed TOML, `/etc/afanctl/afanctl.toml`, strict validation, unknown-key **warnings** (not errors), every key documented with its true default. Exactly five keys:

```toml
[thresholds]          # °C, integers
high = 66             # ramp starts here; low is derived: high - 3
max  = 86             # full speed from here; guard: max <= 95 (Tjmax 100 - 5)
[curve]
min_rpm = 1200        # clamped to >= fan1_min at load
max_rpm = 6200        # clamped to <= fan1_max at load
[poll]
interval_s = 1        # >= 1
```

Validation rejects: `high >= max`, `max > 95`, `min_rpm >= max_rpm`, `interval_s < 1`, non-integer temps — with the key name and the fix in the error (kills H3 and mbpfan's missing≡0 trap by construction). Install ships `afanctl.toml.default`; pacman `backup=` protects user edits (kills H5). Safety tunables (`VERIFY_TOLERANCE_RPM=150`, `SENSOR_LOSS_POLLS=3`, `OVERSHOOT_POLLS=3`, `SLEW_MAX_RPM_PER_POLL=750`, `WRITE_FAIL_FALLBACK=3`, retry `K=3`) are named constants in one place (`policy.rs`/`supervisor.rs` consts) — not config.

### R7 — Observability (P0)

- Logging via `tracing` → stderr (journald captures): info = mode changes, escalations, fallbacks, watchdog events; debug = per-poll detail; errors are **never** swallowed.
- Daemon publishes `/run/afanctl/state.json` (schema `afanctl.state.v1`, Appendix B) every poll — this is how `status` reflects live daemon state without IPC, and how the plugin renders RPM/mode.
- `status` merges: config + provenance, state file (if daemon alive), direct sysfs reads (fan rpm/mode even when daemon down).

### R8 — Plugin-facing surface (P1)

- **Command file** `/run/afanctl/cmd.json` (schema `afanctl.cmd.v1`, Appendix B): `{"mode": "observe"|"curve"|"hold", "rpm": <u32, hold only>}`. Written atomically (tmp + rename) by the CLI verbs, root-owned, `RuntimeDirectory=afanctl`. The daemon re-reads it every poll, validates (unknown mode / out-of-range rpm → log + ignore, keep previous mode), and applies it through the same `smc` write-verify path. Daemon death still → L2 → AUTO regardless of commanded mode.
- **Preset mapping for omafan** (documented, not implemented in afanctl): auto → `observe`; low → `hold fan1_min`; med → `hold ~4000`; high → `hold ~5800`; full → `hold fan1_max`; off → `hold fan1_min` (true off impossible via sysfs — Q2); slider → `hold <rpm>`.
- **`status --json`** is the plugin's render feed (stable, versioned schema).
- Packaging ships an optional **polkit rule** granting passwordless `afanctl status|observe|curve|hold` to wheel users — via a pkexec action file (`org.freedesktop.policykit.exec`) pinned to the installed binary path, mirroring the mac-fans helper pattern the plugin ecosystem already uses. The rule is narrow: verb allowlist only, no arbitrary args for `hold` beyond the rpm integer.

### R9 — Packaging & systemd (P0)

- `packaging/afanctl.service` (Appendix D), installed disabled by default (safe: nothing changes until the user enables it; and it starts in observe anyway).
- PKGBUILD: builds release, installs binary + unit + `afanctl.toml.default` + polkit rule + `backup=('etc/afanctl/afanctl.toml')`. AUR name `afanctl` (re-verify Q1 before submit).
- `makepkg -si` must install cleanly on this box (acceptance).

### R10 — Testing (P0; see also §9)

- **Policy table tests** replaying the debate's traces: 80 °C plateau × 50 polls converges to f(80) (H2 dead); {92,60,88} acts on 92 (H1 dead); descending approach converges *down* with no ratchet; hysteresis no-flap sweep; sensor-loss streak → `ReturnToAuto`; overshoot guard; hold-with-hot-core escalates.
- **Mock backend + fixture sysfs tree** (`tests/fixtures/sysfs/`): every `smc` behavior (round-trip verify, drift detection, failed reads, mode flip) tested without hardware.
- **Integration tests** run the real binary with `--sysfs-root` against fixtures: `once`, `hold`→state file→daemon applies, `selftest-panic` leaves fixture `fan1_manual == 0`, L1 re-assert on induced drift.
- **Hardware-in-loop tests** behind `--features hw` **and** env `AFANCTL_HWTEST=1` **and** real applesmc present — otherwise skipped. These are the only tests that may touch `/sys`, and they never run in CI or by default.
- `status --json` output validated against the versioned schema in a test.

### R11 — Non-functional (P0)

- Rust, edition 2021, std-only threading (no async, no tokio), ≤ ~1,400 LOC product code (core ≈ 1,000 + plugin surface ≈ 300; per-module budgets in §7). Exceeding a module budget by >20% = stop and report, don't silently sprawl.
- Dependency allowlist (nothing else without planner approval): `serde`, `toml`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber`, `libc`. No clap (hand-rolled arg parser, ~80 LOC — small verb set, exact grammar in the task card, nonzero exits). `sd_notify` is hand-rolled (~30 LOC, unix datagram to `$NOTIFY_SOCKET`).
- No `unsafe` outside `safety.rs` (the L2 path) — grep-gated.
- Performance: poll loop < 0.1% CPU at 1 Hz (use `sleep` with timer-slack tolerance; no busy waits), RSS < 5 MB, binary < 2 MB release.
- Exit codes: 0 success; 1 runtime failure; 2 usage/CLI error. Documented in `-h`.

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
    #[error("invalid value from {path}: {0}")] InvalidValue { path: std::path::PathBuf, value: String },
}

pub trait Smc: Send {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError>;
    fn read_fan(&self) -> Result<FanState, SmcError>;
    fn hw_min_rpm(&self) -> u32;
    fn hw_max_rpm(&self) -> u32;
    /// Write + read-back-verify (R1 invariant). Returns the verified rpm.
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
    /// L1 re-assert → write state.json → watchdog ping. Returns the report (testable).
    pub fn step_once(&mut self) -> StepReport;
    /// Foreground loop (systemd Type=notify). Installs L2, notifies READY, runs forever.
    pub fn run(&mut self) -> !;
}
pub struct RuntimePaths { pub cmd: std::path::PathBuf, pub state: std::path::PathBuf }

// ---- safety.rs — the ONLY module allowed `unsafe`
/// Pre-open fd + install panic hook and raw SIGSEGV/SIGABRT/SIGTERM/SIGINT handlers that
/// write b"0" (AUTO) to the fan manual file in a single write(2). Async-signal-safe only.
pub fn install_death_path(panic_fd: i32);
/// Deliberate panic for `selftest-panic` (proves L2; verified by fixture/hw tests).
pub fn arm_test_panic() -> !;

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
  "daemon": { "running": true, "mode": "curve", "watchdog_armed": true, "uptime_s": 981 },
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
  "verified": true, "watchdog_pings": 981, "recent_errors": [ … last 5 … ] }
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

## 9. Verification & acceptance criteria (project-level Definition of Done)

1. All four quality gates green in a clean checkout (§8), including `--features hw` skipping cleanly without hardware.
2. Every R10 test exists and passes: the seven policy trace families (plateau, hot-core, descending-approach, hysteresis, sensor-loss, overshoot guard, hold-with-hot-core); smc round-trip/verify/drift/mode-flip on fixtures; integration (`once`, hold→cmd→daemon applies, `selftest-panic` → fixture `fan1_manual == 0`, L1 re-assert on induced drift); `status --json` schema validation.
3. **Supervised hardware gate** (user present; the only real-sysfs session):
   a. `afanctl doctor` — all PASS read-only.
   b. `afanctl doctor --roundtrip` — manual 2 s → AUTO restore verified.
   c. `sudo afanctl selftest-panic` — process dies; `fan1_manual` reads 0; `doctor` confirms AUTO.
   d. `systemctl start afanctl` in observe mode — journal shows `READY` + watchdog armed; 30-min soak, zero errors; `status` reflects state file.
   e. `SIGKILL` the daemon while in curve mode → the unit restarts within ~1 s and the **startup reconcile** restores AUTO (a SIGKILL is uncatchable, so L2 cannot run — the restart is the mechanism that restores the firmware net); verify via `cat /sys/devices/platform/applesmc.768/fan1_manual` (expect `0`) + `status` + journal.
   f. (User-gated) `afanctl curve` + 1-hour soak; then `doctor --compare 600` for the empirical record.
   g. `afanctl hold 3000` from a user shell via the polkit rule — fan goes to ~3000, `doctor` reports the daemon mode and warns when a hold is active (F25), and the overshoot guard is proven by the policy trace tests (§6.R10): the guard requires `OVERSHOOT_POLLS = 3` consecutive polls inside one process, so a stateless `once` invocation cannot reach it (amended 2026-09-14 after the hardware gate).
4. `makepkg -si` builds and installs cleanly on this machine; `systemctl enable` survives reboot test (machine boots with daemon in observe; fan on SMC curve).
5. LOC within budget ±20% (§7); `grep -rn "unsafe" src/` shows hits only in `safety.rs`; `Cargo.toml` deps exactly the allowlist.
6. The six levers held: exact signatures (A), conventions in every instruction (§8), strict file ownership per task, mechanical gates, per-merge review vs the safety checklist (§11.5), DEVIATIONS protocol used or unused-but-present.

## 10. Risks & open questions

| # | Risk / question | Position |
|---|---|---|
| Q1 | Name `afanctl` taken between now and publish | Searched free 2026-09-14 (fanctl/macfanctl/smctl taken; afanctl not found). Re-verify AUR + crates.io before first publish; alternates: `smcfand`, `smcfanctl`. |
| Q2 | Plugin's "off" preset: applesmc sysfs cannot set rpm below `fan1_min` (1200) | Accept: `off` maps to `hold fan1_min` (documented to the plugin as "hardware floor"). True off would need direct SMC key writes — out of scope, will not be added silently. |
| Q3 | `fan1_safe` semantics unknown (reads empty) | Ignored in v1; `doctor` reports its existence; revisit only with upstream evidence. |
| Q4 | applesmc completes its hwmon conversion upstream (today: attribute-less husk at hwmon3) | Absorbed by design: one `smc` module + `doctor` layout check + deliberate coupling to the unit's `ReadWritePaths`. A kernel bump changing layout = one-module fix, detected by doctor. |
| Q5 | hwmon indices are dynamic (coretemp = hwmon4 today) | Discovery walks `/sys/devices/platform/coretemp.0/hwmon/hwmon*`; no hardcoded `hwmonN` anywhere. |
| Q6 | Hold mode thermal liability | Guards are spec'd (clamp, overshoot override, L1/L2 stay active) and tested; `doctor`/`status` surface hold state loudly. |
| Q7 | `StartLimitIntervalSec=0` + crash-loop = noisy 1 Hz restarts | Accepted: noisy-but-safe beats quiet-but-dead. journald rate-limits; a genuinely crash-looping build should fail the supervised gate anyway. |
| Q8 | polkit rule breadth | Allowlist of exact verbs; `hold` accepts only an integer rpm. No `daemon`, no `doctor --roundtrip` passwordless. |

## 11. Orchestration brief (for the planning agent)

**Crew.** Planner: glm-5.3, maximum thinking. Subagents: **glm-5.3-flash (reasoning: high)** — assign every safety-critical, stateful, or `unsafe` task; **deepseek-v4.1-flash** — assign well-specified mechanical surfaces (the specs below are exact enough that this is safe, and the review gate catches drift). Provider for all: opencode-go.

**The six consistency levers** (why parallel multi-model work will converge): (1) §7 signatures fixed before any implementation; (2) §8 conventions block pasted verbatim into every instruction; (3) strict per-task file ownership — no two tasks edit the same file; (4) mechanical gates (fmt/clippy/test) that admit no taste; (5) a review pass per merge against a fixed checklist; (6) DEVIATIONS/QUESTIONS files as the only cross-agent channel.

### 11.1 Sequencing

```
T0 scaffold+contracts ──► T1 config ─┐
                      ├─► T2 policy ─┼─► T5 supervisor ─► T7 doctor ─► T8 integration+packaging ─► T9 review gate
                      └─► T3 smc ────┤        ▲
                           └► T4 safety+notify ┘
                      T6 cli+main (start after T0; final wiring after T5)
```
T1 ∥ T2 ∥ T3 are fully parallel (disjoint files, shared only the frozen contracts). T4 needs T3's fd design but can start its spec'd body in parallel — merge after T3. Merge strictly in dependency order; the planner rebases each task branch onto `main` as its dependencies land, then runs the gates itself before merging.

### 11.2 Task cards

Each card states: goal · files owned · spec references · tests required · done criteria. Every instruction to a subagent = the §8 conventions block (verbatim) + the card + "read `DESIGN.md` first".

**T0 — scaffold + contracts** · glm-5.3-flash (high) · files: `Cargo.toml`, all `src/*.rs` stubs, `DESIGN.md`, `DEVIATIONS.md`, `QUESTIONS.md`, `tests/` skeleton, fixture tree skeleton, a `check.sh` running the four gates.
Create the crate with every Appendix-A signature present as compiling stubs. Each stub body is `unimplemented!("owned by T<n>")` — the crate must compile, every gate must pass, and any stub that gets called fails loudly with the task that owes it. `DESIGN.md` = verbatim copy of PRD §7+§8. Fixture tree mirrors §2.1 exactly (hwmon4 under coretemp.0, husk hwmon3 under applesmc, fan files with the real min/max). Done: gates green; every signature present exactly as written; each stub tagged with its owning task.

**T1 — config** · deepseek-v4.1-flash · files: `src/config.rs` (+ its unit tests), `packaging/afanctl.toml.default`.
Implement R6 + Appendix A `Config`/`ConfigError` exactly. Tests: defaults; every validation rejection with the key+fix in the message; unknown-key warning collection; missing file → defaults; clamping against hw tuple; the H3 landmine (`high == max`) rejected. Done: gates + tests green.

**T2 — policy** · glm-5.3-flash (high) · files: `src/policy.rs` (+ unit tests), `tests/policy_traces.rs`.
Implement R2 + `Controller` exactly. Tests = the seven trace families from R10 (write them as data tables: input readings → expected decision sequences). Include the arithmetic test: `target(high..max)` linear endpoints exact at boundaries. Done: gates + all traces green.

**T3 — smc** · glm-5.3-flash (high) · files: `src/smc.rs` (+ unit tests), `tests/fixtures/sysfs/**` (completing T0's skeleton).
Implement R1 + `SysfsSmc`/`MockSmc`. Discovery walks (Q5); read-back-verify invariant; clamping; `panic_fd`. Tests on fixtures: round-trip verify; a write that "doesn't take" → `VerifyFailed` after K retries; failed/short reads; mode flip; sensor outlier rejection; layout-change detection hook (for doctor). Done: gates + tests green; no path strings outside this file.

**T4 — safety + notify** · glm-5.3-flash (high) · files: `src/safety.rs`, `src/notify.rs` (+ unit tests where testable).
Implement L2 exactly as R4/Appendix A: pre-opened fd, panic hook, raw handlers, single `write`. Budget real care here (the debate called it "the trickiest ~20 lines"); document the async-signal-safety argument on every `unsafe` block. `arm_test_panic` for the hidden verb. `notify.rs`: READY/WATCHDOG/STATUS via `$NOTIFY_SOCKET` datagram, no-op-false when unset. Testable pieces (notify socket path handling, fd validity) tested; the signal path itself is proven in T8's `selftest-panic` fixture test and the hw gate. Done: gates; `unsafe` confined here; SAFETY comments complete.

**T5 — supervisor** · glm-5.3-flash (high) · files: `src/supervisor.rs` (+ unit tests).
Implement R3 modes, the poll loop order **exactly**: read cmd file → validate/apply mode → read sensors → controller step → act via smc (verify) → L1 re-assert → write `state.json` → watchdog ping → sleep. `step_once` returns `StepReport` (testable with `MockSmc`). L1 counters → fallback-to-AUTO + monitor-only degradation after `WRITE_FAIL_FALLBACK`. Tests with mocks: mode transitions via cmd file; invalid cmd ignored+logged; L1 drift re-assert; fallback path; hold clamped; state file written correctly. Done: gates + tests green.

**T6 — cli + main** · deepseek-v4.1-flash · files: `src/cli.rs`, `src/main.rs`.
Hand-rolled arg parser: exact grammar = R5's verb table + globals `--config`, `--sysfs-root`, `--json` (status/doctor), `--at-temp`, `--dry-run`, `--mode` (daemon), `--roundtrip`, `--compare <s>` (doctor). Exit codes per R11 (0/1/2); unknown flag → usage + exit 2. Human output formats for status/doctor per Appendices B/C. `main.rs` = wiring only. Tests: parser table (every verb, every error case, exit codes); `--version`. Done: gates + parser tests green; no logic in main.rs beyond wiring.

**T7 — doctor** · deepseek-v4.1-flash · files: `src/doctor.rs` (+ unit tests).
Implement R5's check list + `--compare` (sample t_eff + SMC rpm in observe, simulate curve over the same trace, table + stats, Appendix C format). All read-only except `--roundtrip` (2 s manual → restore → verify, behind a confirm flag). Uses smc's layout-change hook (Q4). Tests: each check against fixture trees (healthy, missing driver, unwritable, bad config, changed layout); compare math on a recorded trace. Done: gates + tests green.

**T8 — integration + packaging** · deepseek-v4.1-flash · files: `tests/integration.rs`, `tests/schema.rs`, `packaging/afanctl.service`, `packaging/PKGBUILD`, `packaging/polkit rule`, `README.md`.
Integration: spawn the real binary with `--sysfs-root` fixtures — `once` decision output; `hold` → cmd file → daemon (`daemon` in a spawned process against fixture) applies within one poll; `selftest-panic` exits nonzero and fixture `fan1_manual == 0`; L1 re-assert on induced drift; `status --json` validates against `afanctl.status.v1`. Packaging per R9/Appendix D + polkit rule per Q8. README: install, safety model, verbs, plugin preset mapping (R8). Done: gates + integration green; `makepkg` runs (full `makepkg -si` is part of the hw gate).

**T9 — review gate** · glm-5.3-flash (high) (or the planner itself) · files: none (may open fix tickets, not fix directly).
Adversarial review of the whole tree against: every §6 requirement met; every §9.1–9.5 criterion evidenced; the mbpfan defect classes each impossibility-tested (C1/C2/H1/H2/H3); conventions audit (unsafe/allowlist/LOC/newtypes); README accuracy. Output: PASS, or a prioritized defect list that loops back as fix tasks (same conventions, same gates). This gate precedes the supervised hardware gate (§9.3), which requires the user present and is therefore *outside* the subagents' scope — the planner should end its plan by handing §9.3 to the user as a checklist.

### 11.3 Instruction template (paste for every subagent)

```
ROLE: You are implementing exactly ONE task of the afanctl project (Rust fan supervisor,
A1708 MacBook). You are part of a multi-agent build; other agents own other files.

READ FIRST (in order): DESIGN.md (contracts + conventions — BINDING), then your task card below.
You may also read, never edit, any other file in the repo.

YOUR TASK: <card: goal / files owned / spec refs / tests required / done criteria>

CONVENTIONS (verbatim from DESIGN.md §8): <paste §8 block>

RULES:
- Touch ONLY the files listed above. Everything else is read-only.
- Use EXACTLY the signatures in DESIGN.md Appendix A. Need a change? Record it in
  DEVIATIONS.md (old → new → why → affected tasks), keep everything else working, and flag
  it in your summary. Never silently alter a public item.
- Ambiguity? Write it to QUESTIONS.md, do the unambiguous rest, say so in your summary.
  Do not invent spec.
- NEVER write to the real /sys. All tests use tests/fixtures/sysfs via --sysfs-root.
- No new dependencies, no unsafe (unless your card says safety.rs), no unwrap/expect/panic
  outside tests, LOC budget: <per card>.

DONE WHEN (run and show output of all): cargo fmt --check;
cargo clippy --all-targets --all-features -- -D warnings; cargo test;
cargo test --features hw  (must skip cleanly); plus your card's own checks.

FINAL OUTPUT: files changed, gate outputs, tests added, deviations/questions logged,
and anything you deliberately left for a later task.
```

### 11.4 Merge protocol (planner-owned)

One branch per task (`t0-scaffold`, `t1-config`, …). Planner merges in §11.1 order; before each merge it runs the four gates on the merged tree; any red = bounce back to the task's agent with the failing output. After T5 and after T8, run an intermediate conventions audit (unsafe-grep, dep-allowlist diff, LOC count vs budget) — cheap catches before the expensive T9 gate.

### 11.5 Per-merge review checklist (T9 uses the full form; planner uses items 1–5 per merge)

1. Signatures match Appendix A (or a DEVIATIONS entry exists and is consistent).
2. Every write path goes through verify; no logical state from unverified writes.
3. Every new failure path ends in AUTO / refusal-to-start / loud log — never silence.
4. Tests added cover the card's behaviors; no test writes real sysfs.
5. Conventions: fmt/clippy clean, no stray deps, error messages name key+fix.
6. (T9 only) Each mbpfan defect class (C1 C2 H1 H2 H3 H4 H5 H6) mapped to the test that proves it dead.

---

## 12. Deferred / future work (do not build in v1)

`auto-intervene` mode · multi-fan control · direct-SMC-key access (true fan-off) · persisted mode across reboots (today: observe after every boot, by design) · man page · crates.io/AUR publication (after Q1 re-check and the hw gate) · omafan plugin itself (separate project; consumes R8) · the three-line `buf[-1]`/`buf[16]` fix PR to mbpfan upstream (independent good citizenship — anyone can send it; do not couple it to this project).
