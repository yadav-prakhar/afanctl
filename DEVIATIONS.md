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
