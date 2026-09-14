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
