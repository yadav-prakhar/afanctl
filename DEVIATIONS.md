# DEVIATIONS

Append-only log of proposed changes to the frozen contracts (DESIGN.md /
PRD §7 Appendix A). Format: old → new → why → affected tasks. A deviation is
not a change — it is a proposal; the planner decides. Never silently alter a
public item.

---

## T0 — scaffold + contracts

### D1: `InvalidValue` error attribute name (thiserror format string)

- **Old (Appendix A):** `#[error("invalid value from {path}: {0}")] InvalidValue { path: ..., value: String }`
- **New:** `#[error("invalid value from {path}: {value}")] InvalidValue { path: ..., value: String }`
- **Why:** the tuple-index format `{0}` does not compile for a struct-variant
  in thiserror ("invalid reference to positional argument 0"); named-field
  formatting is the only valid rendering of the identical variant shape.
  Signature (variant name, fields, types) is unchanged — display string is
  byte-identical.
- **Affected tasks:** T3 (raises it), T7 (displays it).

*(no other deviations; all other Appendix A signatures reproduced exactly)*

### N1 (note, not a deviation): transitive crates in Cargo.lock


`Cargo.lock` contains `valuable` and `windows-sys`/`windows-link`. Verified
these are cfg-gated optional arms of allowlisted deps (`tracing-core`'s
`[target.'cfg(tracing_unstable)'.dependencies.valuable]`, `nu-ansi-term`'s
`[target.'cfg(windows)'.dependencies.windows]` aliased as `windows-sys`) and
are NOT compiled on this platform (`cargo build -v` shows neither; `cargo
tree -i valuable` prints nothing). Direct dependencies are exactly the R11
allowlist: `serde`, `toml`, `serde_json`, `thiserror`, `tracing`,
`tracing-subscriber`, `libc`. No deviation, logged for the T9 allowlist audit.


### RULING (orchestrator, 2026-09-14) on D1: ACCEPTED
DESIGN.md amended accordingly (sole governance exception; the amended line is the
binding contract going forward). Validated live: `{0}` fails to compile for a
named-field variant, `{value}` compiles and renders identically. Propagation:
T3/T7 instructions carry a RULINGS preamble noting this is already applied — no
action needed by them. PRD.md §7 Appendix A left untouched (historical source;
DESIGN.md is the binding contract per PLAN.md header).

### RULINGS (orchestrator, 2026-09-14) — Wave 1 adjudications

- **D-T6-1 ACCEPTED:** `doctor::run` signature amended to `pub fn run(roundtrip: bool,
  compare_secs: Option<u64>, json: bool) -> i32`. DESIGN.md updated (governance exception).
  Root cause: PRD R5/Appendix C mandate `doctor --json` but Appendix A omitted the channel.
  Propagation: T7 card carries the ruling; T6a's `run_doctor()` wired in PHASE (b).
- **T3 N2 ACCEPTED (note):** `SysfsSmc::layout_changed(&self) -> bool` as an inherent
  method is the sanctioned shape for the layout hook; T7 consumes it.
- **T1 Q-T1-1 ACCEPTED as spec ruling:** a present config file with a missing key is
  REFUSED (`ConfigError::Invalid` naming key+fix); only a wholly missing file falls back
  to defaults. This is now the spec — T5/T7/T8 rely on it.
- **T6 Q-T6-1 ACCEPTED as spec ruling:** `AFANCTL_RUNTIME_DIR` env seam is blessed for
  T5/T8 (default `/run/afanctl`); doc it in README (T8) and use it in all integration tests.
- **T6 Q-T6-2:** deferrals acknowledged — all land in PHASE (b) (T6b card unchanged).
- **LOC overages (T1 config.rs 231, T3 smc.rs 553+334, T6 cli.rs ~650):** noted in ledger;
  none trigger a bounce (mandated feature set; stop-and-report was honored).

## Final Wave 1 status: all rulings propagated; DESIGN.md amended below.
## T3 — smc

### N2 (note, not a deviation): `SysfsSmc::layout_changed` hook added

The T3 card mandates a "layout-change detection hook (for doctor)" but the
frozen Appendix A does not specify its shape. Added as an **inherent method**
(no trait `Smc` change):

- **Why an inherent method:** the `Smc` trait is frozen (MockSmc has no real
  layout) and only a real-backend consumer (doctor, T7) needs it.
- **Shape:** `pub fn layout_changed(&self) -> bool` — true iff discovery had
  to fall back to a `hwmon*` subdir under the applesmc platform dir, OR a
  `hwmon*` dir under the platform dir has grown `fan1_*` attributes
  (conversion in flight).
- **Affected tasks:** T7 (doctor consumes it); no other task touches it.

No Appendix-A signature altered; `SmcError`, `FanMode`, `FanState`,
`SensorReading`, the `Smc` trait, and both constructors are byte-identical
to DESIGN.md (incl. the D1-approved `{value}` display).
---

## T6 — cli + main (PHASE a)

### D-T6-1: `doctor::run` has no channel for the R5 / Appendix-C `--json` flag

- **Old (DESIGN.md Appendix A):** `pub fn run(roundtrip: bool, compare_secs: Option<u64>) -> i32`
- **New (proposed):** `pub fn run(roundtrip: bool, compare_secs: Option<u64>, json: bool) -> i32`
- **Why:** R5's verb table, PRD §11.2-T6 and Appendix C ("`--json` reuses the
  same fields") all define `doctor --json`, but Appendix A's frozen signature
  accepts no such parameter and `doctor.rs` exposes no other entry point. The
  CLI parses the flag correctly but has nowhere to pass it.
- **Interim behavior (PHASE a):** `afanctl doctor --json` is refused loudly on
  stderr with exit code 1 (runtime failure) rather than silently emitting human
  output for a JSON request — errors are never swallowed (§8). The change is a
  one-liner in `run_doctor()` once ruled.
- **Affected tasks:** T7 (owns the output), T8 (`doctor` integration), T6 PHASE (b).

*(no other deviations; the frozen signatures are otherwise used as written)*

---

## T7 — doctor

### N (note, not a deviation): no contract change; private `run_at` test seam

`doctor::run` is implemented exactly as the D-T6-1 ruling amended it
(`pub fn run(roundtrip: bool, compare_secs: Option<u64>, json: bool) -> i32`); no Appendix-A
item was renamed or added. Fixture-level tests drive a **private** `run_at(root, config_path,
roundtrip, compare_secs, json)` helper (not exported), so the frozen public surface is unchanged.
Two spec gaps found while implementing are logged as Q-T7-1/Q-T7-2 in QUESTIONS.md (doctor cannot
receive `--sysfs-root`/`--config`; a cli-owned test reaches real `/sys` with `--roundtrip`).
## T8 — integration + packaging

### D-T8-1: `selftest-panic` never armed L2, so it could not restore AUTO (APPLIED, ratify or revert)

- **Observed (old):** `src/cli.rs` dispatched `Command::SelftestPanic` straight to
  `safety::arm_test_panic()`; `safety::install_death_path` was called only by
  `Supervisor::run`. The hidden verb therefore panicked with no hook installed.
  Empirically, with a fixture copy at `fan1_manual=1`,
  `afanctl selftest-panic --sysfs-root <copy>` exited 101 and left the file at
  `1` — L2 did **not** restore AUTO. This made the card's binding test
  (`selftest-panic` → fixture `fan1_manual == 0`, PRD R10/§9.3c) unsatisfiable
  and the supervised acceptance §9.3c (real hw) would fail.
- **New (applied, minimal):** the dispatch arm is now
  `Command::SelftestPanic => run_selftest_panic(globals)`, a 9-line function that
  opens `SysfsSmc` on `--sysfs-root`, installs the death path when a pre-opened
  fd exists, then calls `safety::arm_test_panic()`. No public signature changed;
  no other behavior was touched (parse/status/once/hold/daemon unaffected).
- **Why T8 touched a file outside its card:** the card's own integration test
  requires the behavior, T6 (the owning task) is merged and not concurrently
  active, and the DEVIATIONS protocol would otherwise leave a safety-critical
  hole unaddressed. Reverting is a one-arm change. **Planner: ratify or bounce.**
- **Interim behavior:** without a writable `--sysfs-root`, the verb still panics
  and exits nonzero (just without a death write) — unchanged fail-loud posture.
- **Affected tasks:** T6 (file owner), T8 (test), T9 (review), supervised gate §9.3c.
- **Evidence:** `tests/integration.rs::selftest_panic_exits_nonzero_and_restores_auto`
  (fixture `fan1_manual=1` → run → exits nonzero ∧ file reads `0`).

### RULING (orchestrator, 2026-09-14) on D-T8-1: RATIFIED (accept)
Verified: selftest_panic_exits_nonzero_and_restores_auto passes; fd-lifetime invariant
documented; minimal scope (9-line arm + dispatch wiring). The verb exists precisely to
prove L2; without arming the death path the §9.3c gate and R10 test are unsatisfiable.
File-ownership breach acknowledged as justified (T6 owner inactive; safety-critical hole).
DESIGN gains a doc note: selftest-panic arms L2 against the --sysfs-root backend.

## FX-A — T9 fix ticket A (F1, F2, F7, F8, F9, F12)

### D-T9-F7: `status` `uptime_s` renders pings × interval_s (approximation chosen)

- **Choice:** the simpler no-signature-change option — `status --json`
  computes `uptime_s = watchdog_pings × interval_s` (saturating multiply;
  interval from the config as loaded). No field renamed, `state.json` schema
  untouched.
- **Approximation:** the daemon pings once per poll, so this approximates
  wall-clock uptime; per-poll write/verify time is not counted.
- **Open item:** the ticket asked for the approximation to be documented in an
  Appendix B comment, but DESIGN.md is outside this ticket's file list — the
  Appendix B comment needs adding by the DESIGN owner (flagged to the
  orchestrator).

### D-T9-F9: "/sys" + default-config-path duplication resolution

- The duplicated `"/sys"` constant is gone: `doctor::run` now receives
  `sysfs_root` explicitly (D-T9-F2), so only `cli.rs` holds
  `DEFAULT_SYSFS_ROOT`.
- The default config path (`/etc/afanctl/afanctl.toml`) remains a private
  const in BOTH `cli.rs` (parse/usage default) and `doctor.rs`
  (`config_path = None` fallback). `smc.rs` exports no consts and is T3's
  read-only file; adding a new public item would require a DESIGN amendment,
  so per the ticket the param-only shape was kept. Drift risk accepted and
  noted here.

### N-FX-A-1 (note, not a deviation): README `once` row update belongs to Ticket B

- F1's doc half ("update the README once-row to state that `once` restores
  AUTO on exit") could not be executed here: README is Ticket B's file and
  this ticket's RULES forbid touching it. Flagged for the orchestrator /
  Ticket B (one-line table edit: `once` row → "restores AUTO on exit").
---

## T9F-B — defect tickets F3/F4/F5/F6/F10/F11/F13-doc (branch `t9fix-b`, 2026-09-14)

### N-T9F-B-1 (note): F4 enforces §8 lints via package `[lints]`; cli.rs/doctor.rs/tests/* blame deferred to Ticket A

- **Old:** the §8 clippy gate (`unwrap_used`/`expect_used`/`panic`) was declared
  but enforced nowhere (T9-F4).
- **New:** the three lints live at `warn` in `Cargo.toml [lints.clippy]`
  (edition-2021 package lints), so every `-D warnings` gate enforces them
  end-to-end; `check.sh` gains the explicit expanded gate 2b. Scoped
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` were
  added ONLY inside `#[cfg(test)] mod tests` in B-owned files (config.rs,
  policy.rs, safety.rs, notify.rs, smc.rs, supervisor.rs). `main.rs` needed
  none (no hits — it runs the wiring, no unwraps); safety.rs' pre-existing
  file-level `#![allow(clippy::panic)]` for the sanctioned `arm_test_panic`
  is unchanged. `[lints] rust` left intentionally empty (the §8 bans are
  clippy tool lints).
- **Why:** per the card's RESOLUTION the lints cannot be "allow" globally, and
  the branch must keep the gates green without touching Ticket A's files.
- **Consequence (TODO for Ticket A):** until A lands its per-file attrs,
  expanded-clippy errors are confined to `src/cli.rs`, `src/doctor.rs`
  (A's files, test blocks) and the test targets `tests/integration.rs` (A's)
  plus `tests/schema.rs` / `tests/policy_traces.rs` (outside both cards — B
  may not touch `tests/` per the task card; recommend A or the planner add
  the same one-line attrs there). check.sh gate 2b encodes exactly this
  blame rule and fails on any error naming any other file.
- **Affected tasks:** T9F-A (attrs in its own files; recommended stretch:
  tests/*.rs attrs).

### N-T9F-B-2 (note): F5 layer 2 hardened all four constant-multiply sites, not the two cited

The ticket/review cites policy.rs:195 and :204; the identical
threshold-×1000 pattern also exists at :207 and :210 (`max_c`, `high_c`).
All four are converted to `saturating_mul` (plus `saturating_sub` for
`max − 1` in `overshoot_check`); leaving two same-class sites raw would be an
incomplete fix of the same defect. Saturation degrades toward cooling: an
absurdly low threshold lands in the max zone → the fan goes to max. No
public signature changed; `Config`/`ResolvedConfig` shapes untouched — F5's
band validation reuses `ConfigError::Invalid { key, reason, fix }` as the
card mandates.

### N-T9F-B-3 (note): config.rs growth on top of a ledgered overage

F3 (watchdog-coupling constants + rejection) and F5 (0..=95 °C band for
high and max) add ~45 lines incl. their mandated tests to `src/config.rs`,
already flagged over the §7 budget (N-T1-1). Same defect-fix scope as the
card; nothing trimmed, no public item added beyond the two constants the
card itself mandates (`WATCHDOG_UNIT_SEC`, `MAX_INTERVAL_S`). Reported, not
absorbed silently (§8).

## F14 — startup reconcile + startup evidence line (branch `fix/startup-reconcile`, 2026-09-14)

### N-F14-1 (deviation, orchestrator call needed): evidence line cannot name the config source path

- **Required by card:** the startup `tracing::info!` must name the *config
  source path*.
- **Blocker:** the Appendix-A signatures are frozen and `src/cli.rs` is
  read-only for this ticket. `Supervisor::new(smc, cfg, start, paths)` receives
  no config path; `RuntimePaths { cmd, state }` (also Appendix A) has no
  config field; `cli::build_supervisor` is the only place that knows
  `globals.config`. Any route to it — a `RuntimePaths` field, a
  `Supervisor::new` parameter, or a new public setter — is a public-item
  change plus a required one-line edit in a read-only file.
- **Shipped instead:** the line names effective mode, L2 armed, watchdog
  notify path, the daemon-owned `state_file` path, and the hw band. The
  config-source field is **stopped** per the card's signature rule.
- **Proposed change (old → new):** `RuntimePaths { cmd, state }` →
  `RuntimePaths { cmd, state, config_source: PathBuf }`, populated by
  `cli::runtime_paths()`/`build_supervisor` from `globals.config`; the
  evidence line then logs `config_source = …`. Affected: DESIGN.md Appendix A,
  `src/cli.rs` (two one-line edits), `src/supervisor.rs` (one field + one log
  field). Defer to the orchestrator; until then the journal line is missing
  that one field (P5 (e)/(f) evidence impact is minimal — config provenance is
  still visible in `status --json`).

## F19 — settle-window verify + stale-binary doctor check (branch `F19`, 2026-09-14)

### N-F19-1 (deviation, reported per card): product-code LOC over the +180 budget

- **Card budget:** ≤ +180 product-code LOC (tests excluded). Measured: net
  **+223** (policy.rs +14, smc.rs +113, doctor.rs +96; non-test lines only,
  insertions − deletions vs the merge-base commit).
- **Driver, not creep:** both new mechanisms are mandated whole —
  (a) the settle-window write/verify rework (write-once, windowed echo
  polling, single re-issue) touches *both* backends and adds the time-based
  echo-adoption model the mock needs (`set_echo_latency`, per-invariant doc
  comments §8 requires on safety-critical paths), and
  (b) the doctor stale-binary check is a new checklist surface (systemctl
  query, `date`-based local-time conversion because R11 forbids a tz crate,
  mtime resolution with `/proc/<pid>/exe` → `/usr/bin/afanctl` fallback, pure
  classifier). ~35% of the delta is mandated doc/invariant comments (§8:
  public items + safety-critical lines get them).
- **Nothing trimmed:** compacting further (single-letter closures, merged
  branch arms in the settle/verify logic) was rejected — the borrow-light
  explicit shape is what keeps `VerifyFailed` reporting and the
  stale-echo-not-a-failure invariant auditable. F16/F16a semantics unchanged
  (R4). Constants added are exactly the ones the card names plus
  `WRITE_RETRY_MAX = 1` (R3's "re-issued once"), all next to the others in
  `policy.rs`, not config.
- **Per-file §7 check:** unchanged files are untouched; the three touched
  files match their pre-task sizes plus the mandated additions. Reported
  loudly, not absorbed (§8).

## F20 — stall detector re-key + honest AUTO restore + L2 invariant + dwell WARN (branch `fix/stall-and-auto`, 2026-09-14)

### N-F20-1 (deviation, within budget): product-code LOC ledger

- **Card budget:** ≤ +250 product-code LOC (tests excluded). Measured:
  **+158 net** (supervisor.rs +138, policy.rs +10, cli.rs +10 — the additive
  `auto_restore_pending` status field and its human marker).
- No other files touched. `tests/pin_constants.rs` added (tests excluded from
  the budget), superset of T9b finding 6's pinning ask.
- Design note recorded in QUESTIONS.md `N-F20-1`: `l2_absent` is the
  startup-verdict form of R3's guard — the literal per-poll recheck would
  break the frozen MockSmc semantics (Appendix A) the suite is built on.

## F26 — applesmc ABI generation binding (branch `fix/applesmc-kernel-7.3-abi`, 2026-09-21)

### RULING F26 (issue #2): `smc.rs` binds one of TWO applesmc attribute generations, by file probe

- **Old (DESIGN.md Appendix A / PRD §2.1):** the fan attribute names are
  constants — `fan1_output` is the actuator, `fan1_manual` is the mode bit
  with the vocabulary `0 = auto, 1 = manual`, `fan1_max` is writable.
- **New:** `smc.rs` owns a two-entry table (`LEGACY`, `MODERN`) and binds one
  entry at `SysfsSmc::open` by probing for the **mode attribute**, which is the
  discriminator. Legacy (kernel <= 7.2): `fan1_output` + `fan1_manual`,
  Manual = `1`, Auto = `0`. Modern (kernel >= 7.3): `fan1_target` +
  `pwm1_enable`, Manual = `1`, **Auto = `2`**, `fan1_max` read-only, no `pwm1`.
  Every attribute name and every mode token — the L2 restore bytes included —
  is derived from the bound entry. Added public items: `AbiGeneration` (+
  `token()`), inherent `SysfsSmc::abi_generation()` and
  `SysfsSmc::abi_evidence()` (the `layout_changed` precedent, ruling T3 N2).
  `FanMode` stays an enum; only the byte written changes.
- **Why:** kernel commit `94f5081d` ("hwmon: (applesmc) Convert to
  `hwmon_device_register_with_info`", released in 7.3) renamed both attributes
  with **no back-compat aliasing** — the hwmon core generates attribute names
  from the driver's declared bitmask, and the conversion deleted the legacy
  `fan_group[]` table that created the old names. `open()` on `fan1_manual`
  returns `ENOENT` on >= 7.3, so afanctl does not start. Detection is by file
  probe and never by parsing a kernel version string: distributions backport,
  and the attribute set is the only truth. A tree exposing **both** mode
  attributes is refused rather than guessed at, because the AUTO tokens are
  mutually incompatible (legacy `2` means *manual*; modern `0` is `-EINVAL`),
  so a wrong guess is precisely the hazard this ruling exists to prevent.
- **Judgement call the issue asks to be recorded — attribute names in
  log/error strings and `doctor` check names:**
  - **`doctor` check names: UNCHANGED, deliberately.** They are effectively a
    public contract (scripts and the plugin match on them) and a name that
    shifted under a kernel upgrade would be the worst of both worlds. The
    bound generation appears instead in the *evidence details*, plus one
    **additive** check, `applesmc ABI generation`, whose detail string is
    built inside `smc.rs` (R1: `doctor` still spells no attribute name of its
    own). `doctor_reports_the_detected_abi_generation_for_both_generations`
    pins the name list as identical on both generations. Additive check names
    have precedent (`stale-binary`, `daemon mode`).
  - **Log/error strings in `supervisor.rs` and `cli.rs`: made
    generation-neutral NOW, not in Phase 1.** They described the *attribute*
    (`fan1_manual=0`); they now describe the *logical mode* (`fan mode
    verified Auto`). Reason: a message naming `fan1_manual` on a 7.3 machine
    is actively misleading in exactly the bug report where it matters most,
    and the correct attribute name still reaches the user — `SmcError::Write`
    / `::InvalidValue` name the real path, generation-correctly by
    construction. Threading a name out of `smc.rs` instead would have broken
    R1 for zero benefit. Cost: four unit-test assertions on those strings were
    updated. Phase 1's capability model owns any further naming work.
- **Affected areas:** `src/smc.rs` (table, detection, all attribute use),
  `src/doctor.rs` (additive check + details), `src/supervisor.rs` and
  `src/cli.rs` (message wording only), `tests/fixtures/sysfs/modern/` (new
  tree), `DESIGN.md` Appendix A, README/`docs/SAFETY.md`, `CHANGELOG.md`.

## F27 — per-backend L2 safe restore (branch `fix/applesmc-kernel-7.3-abi`, 2026-09-21)

### RULING F27 (issue #3): the L2 restore target is a per-backend descriptor, proven at arm time

- **Old (DESIGN.md Appendix A):** `safety.rs` held
  `static AUTO: [u8; 1] = *b"0"` and `install_death_path(panic_fd: i32)` wrote
  that one byte to a fd the backend pre-opened on `fan1_manual`. `Smc` exposed
  `fn panic_fd(&self) -> Option<i32>`, and a present fd *was* "L2 armed".
- **New:** `safety.rs` gains `SafeRestore { fd, bytes: &'static [u8] }` (with
  `new`/`fd`/`bytes`), `SafetyCapabilities`, and `is_armed()`.
  `install_death_path(restore: SafeRestore)` replaces the fd parameter. `Smc`
  replaces `panic_fd` with `safe_restore() -> Option<SafeRestore>`,
  `probe_safe_restore() -> Result<SafeRestore, SmcError>` and
  `safety_capabilities() -> SafetyCapabilities`. `safety.rs` now contains **no
  attribute name and no restore value**: the backend supplies both.
- **Why:** the compile-time byte is correct for exactly one backend on one
  kernel generation. The same "one byte, one syscall" mechanism, ported by
  renaming a path, yields a silent failure to restore (applesmc >= 7.3 answers
  `-EINVAL` to `0`), a fan pinned at full speed forever (generic hwmon, where
  `0` means *no control*), or a stopped fan on a hot laptop (the `pwm1` duty
  file). L2 ignores write errors by design, so all three are silent. **F26's
  rename is the first case that proves the descriptor is needed, which is why
  the two land together**: the naive port of #2 — new path, old byte — is
  strictly more dangerous than the startup failure it fixes.
- **Async-signal-safety, preserved exactly:** the handler is still ONE
  lock-free atomic load and ONE `write(2)`, with no allocation, formatting,
  locks or path construction. The descriptor is published into leaked
  `'static` storage at arm time (once per arm call, never inside a handler),
  so a single `AtomicPtr` load reaches fd, pointer and length together — no
  torn multi-atomic read. A multi-byte restore (thinkpad_acpi's `"level
  auto"`) is still one `write(2)`; the length is part of the descriptor, and
  the unit test uses a multi-byte payload on purpose so no single-byte
  assumption can creep back.
- **Arm-time probe:** `Supervisor::run()` (and `once`, and `selftest-panic`)
  now write the restore bytes **through the descriptor's own fd** and then
  verify the hardware reports Auto inside `MODE_SETTLE_MS`. Only a proven
  restore arms L2; a failure logs `L2 death path ABSENT` loudly, records it in
  `recent_errors`, and — via the existing F20 R3 guard — refuses every control
  mode. **Narrowing recorded:** R3's "observe writes nothing" is hereby read as
  "observe issues no *control* write". The probe writes only the fail-safe
  value, so it can move the fan toward firmware control and never away; the
  alternative (skip the probe in observe) would leave every observe-started
  daemon permanently unable to accept a `cmd.json` control command, which is
  the plugin's core flow.
- **Startup order:** `run()` samples the inherited fan mode **before** the
  probe, because the probe's own fail-safe write would otherwise erase the
  evidence that a predecessor died owning the fan;
  `reconcile_stale_state(inherited)` takes that snapshot as a parameter.
  RULING F14's order (arm L2 → reconcile → sd_status/READY) is unchanged.
- **`status`/`state` JSON:** an **additive** `safety` object reports the layers
  that are actually armed — `backend` (including the bound ABI generation),
  `l1_verify`, `l2_death_path` (read from `safety::is_armed()`, the single
  truth, not from an intention recorded elsewhere), `l3_watchdog_notify`,
  `hw_watchdog`, and `firmware_auto_on_suspend` as `boolean|null`. Schema ids
  stay `afanctl.state.v1` / `afanctl.status.v1` on the F18/F20/F22 additive
  precedent. `firmware_auto_on_suspend` is **`null` for applesmc**: there is no
  evidence either way, and `false` would be a claim. `hw_watchdog` is `false`
  because applesmc exposes none — arming one where it exists (thinkpad_acpi)
  belongs to the separate hardware-watchdog issue, which this ruling only
  makes *reportable*.
- **Affected areas:** `src/safety.rs`, `src/smc.rs`, `src/supervisor.rs`,
  `src/doctor.rs`, `src/cli.rs`, `tests/{integration,schema,pin_constants,settle_window}.rs`,
  `DESIGN.md` Appendix A + B, `SECURITY.md`, `docs/SAFETY.md`, `CHANGELOG.md`.
