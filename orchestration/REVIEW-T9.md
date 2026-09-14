# T9 — Adversarial Review Gate report

**Reviewer:** fresh-context adversarial gate (T9), branch `t9-review`, 2026-09-14.
**Scope:** whole tree read (DESIGN.md, PRD §1–§12, DEVIATIONS.md, QUESTIONS.md, every file in
`src/` (10), `tests/` (3), `packaging/` (4), fixture tree, README); all commands below were run
by the reviewer and are reproducible. READ-ONLY: nothing outside this report was modified;
fixture tree untouched; `git status` clean at report time.

---

## VERDICT: FAIL — 1 CRITICAL, 4 MAJOR, 7 MINOR defects (each reproducible below)

The mechanical gates and the defect-class test mapping are in excellent shape (§A, §B), but the
gate cannot PASS while a P0 verb normal-exits into the exact end-state the project exists to
make impossible (F1), a CLI verb silently ignores its documented globals (F2), and a legal
config turns L3 from a safety net into a crash-loop (F3).

---

## A. Mechanical gates — all green (evidence, this branch, this day)

| Gate | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass (0 warnings) |
| `cargo test` | 98 unit + 4 integration + 7 policy-trace + 4 schema = 113 passed, 0 failed |
| `cargo test --features hw` | 118 passed (hw_guard compiles; 0 ignored — skips by env-check by design) |

## B. MBPFAN DEFECT-CLASS MAP (§11.5 item 6) — 7 of 8 classes proven dead; 1 hole

| Class | Proved dead by (test in repo) | Status |
|---|---|---|
| **C1** crash/hang/exit leaves fan manual, net off | `tests/integration.rs::selftest_panic_exits_nonzero_and_restores_auto` (panic→L2→AUTO); `::hold_cmd_is_applied_by_daemon_within_one_poll` (SIGTERM→L2→AUTO); `src/supervisor.rs::fallback_after_write_failures_degrades_to_monitor_only` (write-failure→AUTO+monitor-only); unit `Restart=always`+`StartLimitIntervalSec=0` (Appendix D verbatim in PKGBUILD) | **HOLE — F1**: the live `once` verb normal-exits with `fan1_manual=1`, no L2 armed, no AUTO restore |
| **C2** `buf[-1]`/`buf[16]` stack corruption on bad reads | dead by construction: safe Rust; `unsafe` confined to `safety.rs` (`grep -rn unsafe src/` → only safety.rs); every parse validated. Tests: `src/smc.rs::fixture_invalid_fan_values_are_invalid_value_errors`, `::fixture_outlier_and_failed_reads_rejected` | proven |
| **H1** averages sensors | `src/policy.rs` `absorb()` (max fold, policy.rs:167-189) + `tests/policy_traces.rs::trace_2_hot_core_trio_acts_on_hottest` ({92,60,88} acts on 92) | proven |
| **H2** direction-gated ratchet freezes on plateau | `trace_1_plateau_80c_converges_to_f80`; `trace_3_descending_approach_no_ratchet`; `trace_4_hysteresis_no_flap` | proven |
| **H3** `high==max` passes validation → UB | `src/config.rs::rejects_high_equal_to_max_h3_landmine` + `src/policy.rs::linear_endpoints_exact` | proven |
| **H4** config-reload hazard | deleted by scope: no reload path exists — config is read exactly at startup (`src/cli.rs:362-366`, `src/doctor.rs:336`); `grep -rn "Config::load" src/` shows no per-poll read | proven (by absence, PRD non-goals) |
| **H5** upgrade clobbers user config | `packaging/PKGBUILD:16` `backup=('etc/afanctl/afanctl.toml')` + `afanctl.toml.default` + `src/config.rs::present_but_invalid_file_is_refused`, `::rejects_missing_key_naming_it` | proven |
| **H6** silent write failure | read-back-verify K=3: `src/smc.rs::mock_write_not_taking_fails_verify_after_k_retries`, `::fixture_write_not_taking_fails_verify_after_k_retries` (both prove no commit from unverified write + exact retry count); supervisor fallback test above | proven |

Note (not a defect): the raw-signal half of L2 (SIGSEGV/SIGABRT) is unprovable in-process; the
panic-hook half is proven (`src/safety.rs::l2_panic_hook_writes_auto_and_negative_fd_is_ignored`),
SIGTERM is proven by integration, and the raw arms are covered by the supervised gate §9.3c.
Acceptable remainder per PRD §11.2-T4.

## C. DEFECTS (prioritized; every item reproducible by the stated command)

### F1 — CRITICAL (safety/contract): `afanctl once` normal-exit resurrects the C1 state: fan left MANUAL, L2 never armed, no AUTO restore

- `src/cli.rs:394-439` (`run_once` → `step_once_report`): builds a `Supervisor` in
  `RunMode::Curve` and calls `step_once()` directly. `install_death_path` is called only from
  `src/supervisor.rs:198-215` (`run()`) and `src/cli.rs:527-535` (`run_selftest_panic`) — never
  on the `once` path. Nothing restores AUTO at exit.
- Empirical (fixture copy, `--sysfs-root` tests/fixtures, tempdir runtime): after `once` exits 0,
  `fan1_manual` reads **1** — and `tests/integration.rs:244` *locks this in* (asserts
  `fan1_manual == "1"`). This is precisely mbpfan's C1 end-state ("fans in manual at the last
  written speed with the SMC safety net disabled") reached by **normal, successful operation**
  of a P0 verb, against default `--sysfs-root /sys`.
- Contract violated: PRD Goal 1 ("C1 … impossible by construction"), R4 universal fail-toward-AUTO
  policy, §11.5 item 3 ("every new failure path ends in AUTO — never silence"). A crash *between*
  `enter_manual` and exit has no L2 net either (same hole).
- README is also inaccurate here: the verb table does not warn that live `once` leaves the fan in
  manual mode (README "Command line", `once` row).
- **Repro:**
  ```sh
  cp -a tests/fixtures/sysfs/devices /tmp/once-fix   # writable copy; run as non-root
  cargo run -q -- once --config /nonexistent.toml --sysfs-root /tmp/once-fix
  cat /tmp/once-fix/devices/platform/applesmc.768/fan1_manual   # → 1 (stays 1 forever)
  ```
- Fix direction (for the ticket, not applied): restore AUTO via the verify path before `once`
  exits (or arm L2 + exit-panic semantics is unacceptable — restore is the right shape), and
  update the integration assertion accordingly.

### F2 — MAJOR: `doctor` silently ignores `--sysfs-root` and `--config` (Q-T7-1 never ruled, never fixed)

- `src/cli.rs:301-305`: `run_doctor(json, roundtrip, compare_secs)` receives no globals;
  `src/doctor.rs:31-34` hardwires `DEFAULT_SYSFS_ROOT="/sys"`, `DEFAULT_CONFIG="/etc/…"`.
  `run_at` exists but is reachable only from doctor's own unit tests.
- Empirical: with `--sysfs-root tests/fixtures/sysfs` given, doctor reported `t_eff 69.0 C` /
  `1200..7200 rpm` from the **real live /sys** (fixture says 45.0 C) — and `--config
  /nonexistent.toml` was equally ignored (provenance silently "real path").
- Contract violated: R5 makes `--sysfs-root` a *global* redirect for **all** sysfs access
  (R1: "how the entire test suite … run against fixtures"); a refusal would be loud, this is
  silence (§8: "errors: … name the path/key and the fix", never silent divergence between
  documented flag and behavior). Also leaves F1's supervised-gate doctor checks un-testable
  against fixtures.
- **Repro:**
  ```sh
  cargo run -q -- doctor --sysfs-root tests/fixtures/sysfs --config /nonexistent.toml
  # output shows the live machine's temps/min-max, not the fixture's (45.0 C)
  ```

### F3 — MAJOR: config accepts `interval_s > 15`, which starves the fixed `WatchdogSec=15` and converts L3 into a 1 Hz SIGABRT crash-loop

- `src/config.rs:177-179` rejects only `interval_s < 1`; no upper bound.
  `packaging/afanctl.service:9` pins `WatchdogSec=15`. `step_once` sends exactly one
  `WATCHDOG=1` per poll (`src/supervisor.rs:182-184`), so any `interval_s ≥ 15` (observe,
  curve, or hold) means the watchdog expires → SIGABRT → L2 → `Restart=always` + `RestartSec=1`
  → restart → killed again, forever (inverse of the corrected C1 mechanism: noisy unsafe loop).
- Empirical: `interval_s = 20` config parses, validates, and drives a poll with exit 0:
  ```sh
  printf '[thresholds]\nhigh = 66\nmax = 86\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n[poll]\ninterval_s = 20\n' > /tmp/slow.toml
  cargo run -q -- once --at-temp 50 --config /tmp/slow.toml --sysfs-root <fixture-copy>   # exit 0
  ```
- Fix direction: validation upper-bound `interval_s` against the watchdog budget (it is a
  compiled-in constant of the *unit*, so a constant guard in `config.rs` naming the key+fix is
  the minimal fix), or move watchdog pings off the poll cadence.

### F4 — MAJOR: the §8 clippy gate ("denies `unwrap_used`/`expect_used`/`panic`, allowlisted per-file") is not implemented anywhere

- DESIGN §8 mandates: "Clippy denies them (`clippy::unwrap_used`, `clippy::expect_used`,
  `clippy::panic` — allowlisted per-file where required)." Neither `Cargo.toml` (no `[lints]`
  section, no clippy config) nor `check.sh` passes those flags, and no file carries a scoped
  allowlist. The declared gate therefore enforces nothing.
- Today's product code is individually clean (grep evidence below), so this is an
  enforcement gap, not current dirt — but §8 is BINDING and the four-gate protocol leans on it.
- **Repro** (153 hits, all in `tests/`, zero in product code):
  ```sh
  cargo clippy --all-targets --all-features -- -D warnings \
    -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic   # → 153 errors, all in tests/
  grep -rn "unwrap()\|expect(\|panic!" src/ \
    | awk -F: '($1!="src/main.rs" && $1!="src/safety.rs") && !/\[cfg\(test\)\]/' \
    | grep -vE "tests::|mod tests"     # → only doc-comment hits; product code clean
  ```
- Fix direction: `[lints]` in `Cargo.toml` with the three lints at `warn`/`deny` plus per-file
  `#![allow]`rs in test-bearing files and `safety.rs`/`main.rs`, so `-D warnings` actually gates.

### F5 — MAJOR: accepted config can drive arithmetic overflow in the controller (debug panic / release wraparound) — violates "config invalid → refuse to start"

- Validation checks `high < max` and `max ≤ 95` but no lower bound. `high = -2000000000`,
  `max = -1999999999` passes (`config.rs:143-181`). `Controller` then computes
  `(self.max_c - 1) * 1000` (`src/policy.rs:195`) and `self.low_c * 1000` (`src/policy.rs:204`)
  — i32 overflow: debug build panics *inside the poll loop*, release build wraps (thresholds
  become garbage; with real sensors t_eff ∈ [0,120] every wrapped threshold test can pin the
  fan at `max_rpm`/`EscalateMax` permanently).
- Empirical: `once --at-temp 80` with that config → `panicked at src/policy.rs:195: attempt to
  multiply with overflow`. For the daemon this means a config the program *accepted* ("refuse
  to start" promised in R4/R6 and README) dies at first hot poll (L2 catches it, L3 crash-loops).
- **Repro:**
  ```sh
  printf '[thresholds]\nhigh = -2000000000\nmax = -1999999999\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n[poll]\ninterval_s = 1\n' > /tmp/bad.toml
  cargo run -q -- once --at-temp 80 --config /tmp/bad.toml   # → panic policy.rs:195
  ```
- Fix direction: validation floor (e.g. sane °C range for high/max naming key+fix) or checked/
  i64 arithmetic in `Controller` (`target_for`, `overshoot_check`).

### F6 — MINOR: `selftest-panic` exits 101, outside the documented 0/1/2 exit set

- R11/README/-h document 0/1/2 only; the deliberate panic exits 101 (`src/cli.rs:527-535`,
  `arm_test_panic`). Nonzero (fine), but undocumented as a third code. One-line comment/README
  note or a hook that maps the probe to exit 1. (Repro: `cargo run -- selftest-panic
  --sysfs-root <fixture-copy>; echo $?` → 101.)

### F7 — MINOR: `status` schema fidelity — `daemon.uptime_s` is actually the poll/ping count; `watchdog_armed` is a proxy

- `src/cli.rs:634` renders `uptime_s` from `state.json.watchdog_pings` and
  `watchdog_armed` from "state file exists". With `interval_s ≠ 1` (legal per config), pings ≠
  seconds (Appendix B: `uptime_s`). Degraded/monitor-only state is also invisible apart from
  `verified:false` + `recent_errors`. Repro: seed `watchdog_pings: 42` with default config →
  `status --json` reports `uptime_s: 42` (`src/cli.rs::tests::status_merges_state_file_and_sysfs_reads`
  locks the aliasing).

### F8 — MINOR: stale panic message reachable from the shipped CLI

- `src/cli.rs:514-522`: the T6-era `catch_unwind` around `doctor::run` (built for the T7 stub)
  survives into the final code; if `doctor` ever panics for a real reason, stderr says
  "doctor: not available yet (T7 pending)" — false and misleading. Delete the catch (or map to
  a truthful message).

### F9 — MINOR: duplicated defaults for `--sysfs-root`/config path across cli and doctor

- `src/cli.rs:23-25` and `src/doctor.rs:31-34` both define `"/sys"` and the default config path.
  R1's knowledge-confinement is honored for the sysfs *tree* (all fan/coretemp paths live in
  `smc.rs` ✓), but this duplicated constant is the concrete drift risk behind F2. Consolidate
  or forward via F2's fix.

### F10 — MINOR: cmd freshness gate can strand a verbatim re-command

- `src/supervisor.rs:242-251`: a re-issued command whose bytes equal the last applied one is
  ignored — including the re-arm case after monitor-only degradation if the plugin re-writes
  the identical hold. Deliberate (tested: `fallback_after_write_failures…` tail) and defensible
  (freshness beats flap), but undocumented in README (R8 says "re-reads, validates, applies");
  document or key the gate on inode/mtime.

### F11 — MINOR: `doctor` as non-root reports FAIL (exit 1) on an otherwise healthy machine

- `SysfsSmc::open` cannot open `fan1_manual` O_WRONLY as uid≠0 → "fan files present & writable"
  and "L2 fd armed" FAIL (`src/doctor.rs:201-243`), so §9.3a ("doctor — all PASS read-only")
  can only be met as root. README never says doctor must run as root. Document, or downgrade
  to WARN when the failure is EACCES for the current uid (root-writability is intact).

### F12 — MINOR: value-taking flags accept `-`-prefixed garbage values

- `src/cli.rs:231-237` (`value`) consumes the next token unconditionally: `--config --json`
  sets config to the literal `--json` and the verb still runs. Cosmetic; repro:
  `cargo run -q -- once --config --json --at-temp 80` behaves as `--config=--json` with default
  output (silent misconfiguration, exit 0).

### F13 — MINOR (weighed as instructed): LOC budgets

Product (non-test) lines vs §7 budgets: `cli.rs` 710/~200, `doctor.rs` 642/~200,
`supervisor.rs` 639/~250, `smc.rs` 553~230, `config.rs` 298/~150. All were stop-and-reported by
their owners (QUESTIONS.md N-T1-1, Q-T3-1, Q-T5-3, Q-T6-3/N-T6-2, N-T7-1) and none bounced per
the ledger ruling; treated as accepted scope with quality risk attached. Total product LOC ≈
3.5k vs R11's "~1,400" for a mandated feature set — the gate flags it; the ledger already owns it.

## D. Conventions audit (§8) — rest clean

- **unsafe:** `grep -rn unsafe src/` → only `src/safety.rs` (3 unsafe blocks; each carries a
  `// SAFETY:` comment naming the async-signal-safety argument; `arm_test_panic` is the sanctioned
  panic with the file-level `#![allow(clippy::panic)]`). Meets §8 + F4 caveat.
- **Dependency allowlist (R11):** direct deps = exactly `serde, toml, serde_json, thiserror,
  tracing, tracing-subscriber, libc`. Transitive `valuable`/`windows-*` cfg-gated, documented
  (DEVIATIONS N1) ✓. No clap, no tokio, no threads in product code (SharedMock Mutex is `#[cfg(test)]`-only) ✓.
- **Newtypes/units:** temps are `MilliC` everywhere across boundaries; rpm bare `u32` ✓
  (`linear_target`/`slew_toward` i64/i32-safe internal math outside F5's constants).
- **Error model:** one thiserror enum per module; every `Invalid` message names key+fix ✓
  (spot-checked config/smc/cli + doctor detail rendering); errors never swallowed on failure
  paths (loud `tracing::error!` + `recent_errors`) ✓.
- **`println!` confinement:** only `cli.rs` ✓ (`doctor.rs` uses `stdout().write_all` — fine).
- **Path confinement (R1):** all fan/coretemp/hwmon layout knowledge in `smc.rs` ✓ (see F9
  for the duplicated `"/sys"` root constant).
- **Module docs + doc comments:** present on every file ✓.
- **Poll order (T5 card):** cmd → sensors → controller → act(verify) → L1 → state.json →
  watchdog ping ✓ exactly, and `step_once` never sleeps ✓ (locked by
  `curve_start_writes_verified_and_state_file_is_correct` asserting `watchdog_pings: 0` on
  first publish).
- **Appendix A signatures:** all reproduced exactly; sanctioned deltas only: D1 (InvalidValue
  `{value}`), D-T6-1 (doctor `json: bool`), D-T8-1 (`selftest-panic` arms L2 — ratified, and
  its behavior is now integrally proven). `layout_changed`, `run_at`, MockSmc setters are
  properly recorded (N2/QT) — no silent public-drift found.
- **Appendix D unit:** byte-equivalent to DESIGN Appendix D ✓; PKGBUILD installs exactly the
  README table (binary, unit, `/etc` config with backup, default copy, polkit rule) ✓; polkit
  rule matches README/Q8 exactly (verb allowlist, wheel/local/active, integer-only hold,
  re-checks program realpath; denies daemon/roundtrip/globals) ✓.

## E. §9 acceptance ledger

| Criterion | Status |
|---|---|
| 9.1 gates in clean checkout | green (§A) — with F4's caveat that the claimed unwrap/panic lint gate is aspirational, not enforced |
| 9.2 every R10 test exists & passes | yes: 7 trace families, smc round-trip/verify/drift/mode-flip/outlier/layout, integration (`once`, hold→cmd→daemon-1-poll, selftest-panic→manual==0, L1 drift re-assert), `status --json` schema validation |
| 9.3 supervised hw gate | **outside T9 scope** (user-present); doctor findings F2/F11 and F1 must be fixed before that gate runs, or §9.3a/9.3c will trip on them |
| 9.4 `makepkg -si` | user-gated; build path verified via cargo (`--locked` resolves; Cargo.lock committed) |
| 9.5 LOC/budgets, unsafe grep, deps | unsafe ✓, deps ✓; LOC over-budget as ledgered (F13) |

## F. Known deviations weighed (per instruction)

- D1, D-T6-1, D-T8-1: verified implemented as ratified; evidence intact; no follow-up needed.
- Fixture incident: repo fixture verified unmodified (`git status` clean; file contents match
  the README'd A1708 values: min 1200 / max 7200 / manual 0).
- LOC overages: see F13 — weigh as quality risk on `cli.rs`/`doctor.rs`/`supervisor.rs`.
- Open QUESTIONS that T9 rules on by finding: **Q-T7-1 → defect F2** (accept-as-is is not
  acceptable: the flag exists in the CLI and lies); Q-T8-1 (PKGBUILD url placeholder) — fine
  until Q1 recheck; Q-T8-2 → polkit rule judged compliant with Q8's intent (noted, no change).

## G. Handoff to the orchestrator

Mechanically re-verify F1–F5 (each has a stated repro) and dispatch fix tickets with the usual
conventions/gates: F1 → `src/cli.rs` + `tests/integration.rs`; F2 → `src/cli.rs` +
`src/doctor.rs` (and DESIGN/DEVIATIONS entry for the `doctor::run` signature); F3 →
`src/config.rs` (+ test); F4 → `Cargo.toml`/`src/*` allowlist attributes; F5 → `src/config.rs`/
`src/policy.rs` (+ test). MINORs may batch into one polish ticket. No work touches main; the
supervised hw gate (§9.3) stays with the user.
