# QUESTIONS

Append-only log of ambiguities or blocking questions. Do not invent spec;
record the question, do the unambiguous remainder, flag it in the summary.

---

## T0 — scaffold + contracts

### Q-T0-1: `Supervisor::run(&mut self) -> !` return type vs. gate `cargo clippy -D warnings`

`run` is specified to "run forever" (`-> !`) and `step_once`/`run` bodies are
`unimplemented!("owned by T5")` in T0. `unimplemented!` panics, which makes
the `-> !` signature need a diverging tail after the macro. Resolved locally
without contract change (the macro's `!` type satisfies the return type), so
**no deviation** — logged only because the T5 implementer should keep the
body shape `unimplemented!("owned by T5")` as the final expression.

### Q-T0-2: `Cargo.toml` `[profile.release]` keys are not in the dep allowlist spirit?

Cargo profiles are toolchain config, not dependencies. Added `strip = true`
and `lto = true` in support of R11's binary < 2 MB requirement. Flagging for
planner awareness; no dependency added. If unwanted, delete the section —
nothing references it.

*(no blocking questions; T0 proceeded on the unambiguous remainder)*

---

## T1 — config

### Q-T1-1: semantics of a *present* config file with a missing key

Appendix A does not say what happens when `/etc/afanctl/afanctl.toml` exists
but omits one of the five keys. Two readings: (a) fill the missing key from its
documented default, or (b) refuse the file. **Chosen: (b) refuse** — every key
is required once a file is present; only a wholly *missing* file falls back to
`defaults()` (PRD R6 qualifies defaults with "missing file"). This is the
fail-safe reading for a thermal controller (a typo'd key can never silently
select a different curve) and matches the module contract "never run on defaults
after a bad edit". The unambiguous remainder is unaffected; no contract change.
`from_toml`/`load` return `ConfigError::Invalid { key: "thresholds.max", … }`
naming the missing key and the fix; a test locks it.

### N-T1-1 (note, not a question): `src/config.rs` LOC vs the §7 budget

§7 budgets `config.rs ~150`; this implementation is 231 non-comment product
lines (298 including doc comments and blanks) — ~54% over. The overage is the
mandated error surface itself: five keys × (missing / wrong-type / out-of-range)
plus the four `check_structural` rejections, each carrying `key` + `reason` +
`fix`, and the unknown-key warning walk (toml/serde derive cannot emit key+fix
messages, so the table is walked by hand). No functionality was trimmed; no
file other than `src/config.rs` grew. Flagged per §8 ("do not absorb silently")
for the orchestrator's LOC audit.
## T3 — smc

### Q-T3-1: smc.rs LOC vs §7 budget (~230)

Measured: 553 lines implementation + 334 lines co-located unit tests
(`#[cfg(test)]`, mandated by the card's "+ unit tests" and §8 conventions)
= 887 total. The ~230 budget (+20% → 276) is exceeded by the mandated
feature set: two backends (SysfsSmc + MockSmc with full fault-injection
surface per DESIGN's "scriptable ... faults, drift, latency"), discovery
walks (Q4/Q5-proof), K=3 write-verify on both writers, outlier window, and
the card-required layout hook. Reporting rather than absorbing silently;
Nothing was cut to fit, and no public item drifted to save lines.
## T6 — cli + main (PHASE a)

### Q-T6-1: runtime-dir override for `cmd.json` / `state.json`

R8 pins `cmd.json`/`state.json` at `/run/afanctl/`, but R5 defines only
`--config` and `--sysfs-root` as globals. Integration tests (T8) therefore have
no sanctioned way to redirect the runtime dir off the real `/run` (root-owned,
shared between tests). PHASE (a) reads `AFANCTL_RUNTIME_DIR` when set and
defaults to `/run/afanctl`. Question: is this env seam blessed for T5/T8, or
should a `--runtime-dir` global / explicit `RuntimePaths` injection be added?
No behavior beyond the default was invented; flagging the seam.

### Q-T6-2: deliberate PHASE-a deferrals

- `once --at-temp <C>` (simulated sensor input) and `--dry-run` are parsed and
  debug-logged but not yet applied — they need the real `StepReport` (PHASE b).
- `hold <rpm>` writes the raw rpm to `cmd.json` without R5's "clamp to hw range
  / reject below `fan1_min`" check, because the hardware min/max come from `smc`
  (T3, still a stub). The daemon re-validates/clamps (R8), so no unsafe write
  occurs; the CLI check lands in PHASE (b).
- `status` output (human + `afanctl.status.v1`) is formatted in full but cannot
  be verified against real data yet; PHASE (b) verifies the config + state file
  + direct-sysfs merge (R7).

### Q-T6-3: `cli.rs` LOC vs the §7 budget (~200)
`src/cli.rs` is ~650 lines (`wc -l`, tests included) at PHASE (a) scope: the
exact R5 parser + parser-table tests, all-verb dispatch, Appendix B/C
formatting, and exit-code discipline. §7 budgets cli.rs at ~200 (±20% = 240).
Flagged rather than silently absorbed (PLAN §2 rule). Options: (a) accept as
PHASE-a scope and let PHASE (b) refactor to fit; (b) split Appendix B/C
formatting into PHASE (b) or its own module. Awaiting the planner's ruling.

### Q-T5-1: `state.json` schema id — card ruling says `afanctl.status.v1`, PRD/DESIGN say `afanctl.state.v1`

The T5 card's "RULINGS SINCE" block states: `state.json 'afanctl.status.v1'`. But PRD R7 says the
daemon publishes `/run/afanctl/state.json` "schema `afanctl.state.v1`, Appendix B", and Appendix B
defines a distinct `afanctl.state.v1` file schema (`afanctl.status.v1` is the `status --json`
output schema, which cli.rs T6 already emits). I implemented `"schema": "afanctl.state.v1"` in
the state file (two matching BINDING sources vs one probable typo in the ruling block), and all
state-file fields follow Appendix B's state example (mode/t_eff_c/target_rpm/last_written_rpm/
actual_rpm/verified/watchdog_pings/recent_errors — note Appendix B's state doc lacks `ts`; I emit
it anyway per the example object). One-word flip if ruled otherwise.

### Q-T5-2: noticed pre-existing flake (T4 territory, not touching)

`cargo test --features hw`: `notify::tests::live_socket_receives_ready_and_status` fails
intermittently (UnixDatagram send/recv race in the test harness itself — plain `cargo test` is
unaffected). Reported here for the planner; `src/notify.rs` is read-only for T5.

### Q-T5-3: `supervisor.rs` LOC vs the §7 budget (~250)

Product code is ~630 lines (tests co-located excluded; §7 budget ~250, ±20% = 300). Drivers: the
binding poll order spans cmd-file parsing/validation (R8), mode-machine entry/leave via
write-verified `set_mode` (R3), act/verify, L1 counter + fallback + freshness gate (R4), atomic
`state.json` publish (R7), `run()` wiring, plus a hand-rolled RFC3339 UTC timestamp (no extra
dependency allowed) and the Appendix-A required testable `StepReport`. Same class of overage as
Q-T6-3 (`cli.rs` 650 vs 200). The alternative — thinning tests — violates §8 ("every behavioral
claim must have a test"). Awaiting the planner's ruling (split? accept?).

## T6 — cli + main (PHASE b)

### Q-T6-2 RESOLVED — every PHASE-(a) deferral closed

All three Q-T6-2 deferrals are implemented and tested (branch `t6-cli-phaseb`):

- `once --at-temp <C>` routes the one-shot step through an in-process `MockSmc`
  seeded with `MilliC::from_c(C)`. It never opens `--sysfs-root` (proved by
  `once_at_temp_never_touches_sysfs`, which passes a nonexistent root). The
  mock's hardware band is the configured curve band (`min_rpm..max_rpm`), since
  reading the real `fan1_min/max` would require the sysfs the flag must avoid;
  documented in `-h`.
- `once --dry-run` computes the controller decision via `Controller::step_curve`
  without constructing a Supervisor: no smc write, no `cmd.json`, no
  `state.json` (`once_dry_run_writes_nothing`,
  `once_dry_run_reads_fixture_without_writing`).
- `hold <rpm>` now opens `SysfsSmc` first and clamps/rejects against the
  discovered band BEFORE writing `cmd.json`: below `fan1_min` → exit 1 with a
  key+fix message (`hold_rejects_below_fan1_min_and_writes_no_cmd`); above
  `fan1_max` → clamped (`hold_clamps_above_fan1_max`); in-range written exactly
  (`hold_within_range_writes_exact_rpm`).
- `status` merge verified by `status_merges_state_file_and_sysfs_reads` /
  `status_missing_state_file_reports_not_running`; `run_status` was split into
  `gather_status` + `status_json`/`status_human` so the config + state + direct
  sysfs merge and both renderings are directly testable.
- `doctor` (T7 stub still `unimplemented!`) now maps the panic to
  `doctor: not available yet (T7 pending)` on stderr + exit 1 via
  `catch_unwind`; verified against the release binary (exit 1, never 101).

### Q-T6-4: `once --json` grammar delta vs R5

R5's verb table lists `--json` only for `status`/`doctor`; PHASE-(b) scope item 1
mandates "'once' … prints the decision line (Appendix B human format; `--json`
when flag present)". Implemented `once --json` as an additive verb flag (JSON
object: `mode`/`t_eff_c`/`decision`/`applied_rpm`/`verified`/`notes`); the
PHASE-(a) assertion that `once --json` was a usage error is updated in place.
If R5 is authoritative and the flag must be refused, this is a one-line revert;
flagged so the planner can rule.

### N-T6-2 (note): `cli.rs` LOC after PHASE (b)

`src/cli.rs` is now ~1160 lines (tests included) vs the §7 budget ~200. The
growth over Q-T6-3's already-flagged ~650 is the mandated PHASE-(b) wiring (real
Supervisor build, mock/dry-run one-shot paths, hold clamp/reject, status merge
split, doctor catch) plus their required tests. Same class as Q-T6-3/Q-T5-3;
reported, not absorbed silently (PLAN §2).

### N-T6-3 (bug found + fixed in the owned file)

PHASE-(b) status verification against the fixture exposed a PHASE-(a) display
bug: human `status` used `format!("{:.1} C", temp_c(m))` although `temp_c`
already returns a formatted `String`, so `{:.1}` truncated it (45.0 °C rendered
as `4`). Fixed by interpolating directly (`"{} C"`), locked by
`status_human_renders_full_temperatures`. JSON output was unaffected.


## T7 — doctor

### Q-T7-1: `doctor::run` has no channel for `--sysfs-root` / `--config`

The frozen signature `pub fn run(roundtrip: bool, compare_secs: Option<u64>, json: bool) -> i32`
(D-T6-1 ruling) carries neither the sysfs root nor the config path, and `cli.rs::run_doctor`
does not forward the parsed `globals` (it calls `crate::doctor::run(roundtrip, compare_secs, json)`).
Consequence: on the CLI path `afanctl doctor --sysfs-root <fixture>` **silently ignores the
flag**; doctor always targets `/sys` + `/etc/afanctl/afanctl.toml`. There is no blessed sysfs
env seam (T8-drops-env-var ruling), so this cannot be worked around.

Resolution used: `run` delegates to a **private** `run_at(root, config_path, ...)` seam, which
is what the unit tests drive against fixture trees/tempdirs. No public item was added or renamed.
Options for the planner: (a) extend the binding signature with `root`/`config` (needs a DESIGN
amendment + a T6 `cli.rs` one-liner), (b) bless an `AFANCTL_SYSFS_ROOT` / `AFANCTL_CONFIG` env
seam, or (c) accept (c) as-is (doctor's real consumer is the supervised hardware gate §9.3,
which runs against default `/sys` anyway). Flagging rather than inventing spec.

### Q-T7-2 (safety finding, cli.rs-owned): cli test drives `doctor --roundtrip` against real `/sys`

`src/cli.rs::tests::doctor_stub_maps_panic_to_exit_1` calls
`run_doctor(true, true, Some(1))`, i.e. `doctor::run(roundtrip=true, compare=Some(1), json=false)`
with the default root `/sys`. Now that the body is implemented this reaches real hardware.
Mitigation already inside `doctor.rs`: `--roundtrip` **refuses to write** unless the L2 fd is
armed (`panic_fd()` is `Some`), i.e. only root + writable `fan1_manual`. On this box tests run as
uid 1000, so `fan1_manual` (`-rw-r--r-- root`) yields `None` and the check fails out before any
`set_mode`/`write_speed` call — verified: `doctor --roundtrip` exits 1 with "refused to write".
Residual hazard: if `cargo test` is ever run **as root on the A1708**, that pre-existing test
would perform a real 2 s manual-mode write + restore. It is not fixable from `src/doctor.rs`
(cli.rs is T6-owned); recommend T6 update the test to the tempdir/`run_at` seam or hw-gate it.

### N-T7-1 (note): `src/doctor.rs` LOC vs the §7 budget (~200)

641 product lines + 419 `#[cfg(test)]` lines = 1060 total. Over the ~200 budget (even at +20%).
The overage is the mandated R5 surface itself: 8 checks with distinct FAIL/WARN/PASS semantics,
the root-taking test seam, the `--roundtrip` write/restore path, `--compare` sampling + pure
comparison math + verdict, and both Appendix-C renderings (human + `--json`) plus their tests.
Same class as Q-T1-N, Q-T3-1, Q-T5-3, N-T6-2; reported per §8, not absorbed silently.

### N-T7-2 (note): write-path audit for the orchestrator emphasis

`grep -nE "write_speed|set_mode|fs::write|OpenOptions|File::" src/doctor.rs` hits only inside
`roundtrip_manual_test` (`set_mode(Manual)`, `write_speed(hw_min)`, `set_mode(Auto)` restore).
No `/sys/` path string occurs in doctor.rs; no `println!`/`print!` (stdout via `write_all`).
Caveat: the read-only checks call `SysfsSmc::open`, which internally pre-opens `fan1_manual`
`O_WRONLY` (no write syscall) — that is `smc`'s established behavior (status/once use it too),
not a doctor write path.

### N-T7-3 (note): `doctor --json` schema id

Appendix C mandates "`--json` reuses the same fields" but names no schema token. Emitted
`"schema": "afanctl.doctor.v1"` to match the repo's versioned-schema convention. One-word change
if the planner wants a different id.
## T8 — integration + packaging

### D-T8-1 applied (not a question): see DEVIATIONS.md §T8 for the `selftest-panic`
L2-arming fix in `src/cli.rs` (a file outside T8's card). Flagged for the
planner's ratify-or-revert. No T8 work depends on a ruling.

### Q-T8-1: PKGBUILD `url` placeholder

R9 specifies the AUR name (`afanctl`) but no upstream URL. `packaging/PKGBUILD`
carries `url="https://example.invalid/afanctl"` as a placeholder pending the Q1
publication re-check (AUR + crates.io). One-line edit when the real URL exists;
`makepkg` does not validate it.

### Q-T8-2: polkit rule allows `status --json` (Q8 says "exact verbs")

R8's plugin story makes `status --json` the omafan render feed, but Q8 says an
"allowlist of exact verbs" and `hold` only an integer rpm. The shipped rule
(`packaging/49-afanctl.rules`) therefore allows exactly `status`,
`status --json`, `observe`, `curve`, and `hold <integer>`; it rejects every
other flag and all globals (`--config`, `--sysfs-root`). pkexec's
`command_line` detail is space-joined and the man page warns against using it
for security checks, and `argv1` cannot constrain the rpm integer — so the rule
re-checks `program` (realpath) and the argv shape, while the CLI independently
parses/clamps the rpm. If Q8 is meant literally (no flags at all), drop the
`--json` arm (one line). The rule is data: `makepkg` installs it; it is not
executed by the test gates.

---

## T9F-B — defect tickets (branch `t9fix-b`)

### Q-T9F-B-1 (coordination — requires a one-line change in Ticket A's file): F3 validation breaks `tests/integration.rs::hold_cmd_is_applied_by_daemon_within_one_poll`

B implemented F3 exactly as carded: `Config` now rejects `poll.interval_s >
MAX_INTERVAL_S` (14; the unit's `WatchdogSec=15` minus a 1 s margin).
`tests/integration.rs` is READ-ONLY for B (Ticket A owns the file), but its
daemon-launch config (around tests/integration.rs:288 area) uses
`interval_s = 30` — now legally refused at startup by the new validation, so
the daemon never runs and the test fails with the intended key+fix message:

    afanctl: invalid poll.interval_s: interval_s (30) would starve the systemd
    watchdog … (fix: set `interval_s` to at most 14 s …)

Required change (Ticket A, one line): drop the integration daemon's
`interval_s` to a legal value ≤ 14 (suggest `1` — the first poll is
immediate, so "applied within one poll" semantics are unaffected). This is
not a defect in F3 — 30 s *is* the crash-loop configuration the card orders
us to refuse. Flagging per §8 (cross-agent protocol; B must not touch A's
files). Until A's line lands, gate 3 on `t9fix-b` is 102 unit + 3/4
integration green with this one coordinated-red test.



## FX-A — T9 fix ticket A

### N-FX-A-1 (non-blocking): README `once` row vs file ownership

T9fixA's defect list asks for a README once-row update (F1 doc half), but its
RULES assign README to Ticket B. Resolved per the cross-agent protocol: the
code + tests implement the restore; the README wording change is recorded in
DEVIATIONS.md `## FX-A` (N-FX-A-1) for Ticket B. No answer needed to proceed.

## F16 — write-verification semantics (RULING F16)

### N-F16-1 (resolved-in-implementation): R3 stall ladder interpretation

R3 reads "…log an error, count a failure ⇒ AUTO + monitor-only", while the
general fallback ladder is `WRITE_FAIL_FALLBACK = 3`. The ticket's required
test 3 pins **a single detected stall ⇒ AUTO + monitor-only after
`STALL_POLLS`**, so the stall detector degrades directly (loud error +
`degrade_to_auto`) instead of riding the 3-strike ladder. Recorded here in
case the orchestrator intended the ladder; test 3 is authoritative in the
meantime.

### N-F16-2 (resolved-in-implementation): echo-not-taken on the static fixture

With echo verification (R1), a static fixture's `fan1_output` file *holds* the
written value, so real "write not taken" can only be fault-injected via
`MockSmc::set_write_stuck` (test 4). The old
`fixture_write_not_taking_fails_verify_after_k_retries` test encoded the F16
defect itself and was rewritten as
`fixture_write_verified_by_register_echo`. The lag/Auto-mirror modeling (R5)
lives in `MockSmc` (`set_tach_lag` / `set_tach_frozen`); the static file tree
is inert by nature and needs no dynamic rpm modeling for the required tests.

### N-F20-1 (resolved-in-implementation): R3's literal `apply_mode` recheck of `panic_fd()` breaks the DESC-mandated mock semantics

R3 reads "make `apply_mode` refuse writing modes when `panic_fd().is_none()`".
Read literally, `apply_mode` re-checks the Smc every poll — but DESIGN.md
Appendix A freezes `MockSmc::panic_fd() == None` for backends *without* a
death path ("None if backend is a mock without one"), and hundreds of
mandated tests drive `curve`/`hold` over such mocks. Implemented instead: the
startup guard's verdict is recorded once on the supervisor
(`l2_absent = panic_fd().is_none()` in `run()`) and `apply_mode` enforces it.
Behaviorally identical on the real backend (the verdict can never change
after startup); the invariant now holds on both channels — startup mode and
cmd.json re-arming — which is R3's acceptance point. Flagged per the
cross-agent protocol; unambiguous remainder implemented.

### N-F21-1 (consequence of RULING F21 R2, cli.rs is read-only on this ticket): `daemon.uptime_s` now counts two pings per poll

RULING F21 R2 pings the watchdog at the start AND at the end of every poll,
so `state.json.watchdog_pings` now advances by 2 per poll instead of 1.
`cli.rs` (read-only for F21) renders `daemon.uptime_s = watchdog_pings ×
interval_s` (D-T9-F7 approximation, whose doc says "the daemon pings once
per poll") — that approximation now reports ≈2× the true uptime. No schema
or signature change is needed; options for the orchestrator: (a) cli divides
by 2 with the constant as the source of truth, (b) supervisor publishes a
separate poll counter, (c) accept and re-document. Flagged per the
cross-agent protocol; not silently "fixed" outside the owned file set.

### N-F21-2 (doc drift, DESIGN.md is read-only on this ticket): Appendix A `step_once` comment and Appendix B example

DESIGN.md Appendix A still describes the poll order as "…write state.json →
watchdog ping" (single end ping); F21 R2 makes it two pings (start + end),
and Appendix B's `watchdog_pings` example should note the two-per-poll
semantics. Source doc not in F21's owned file list — needs a one-line
re-render by the orchestrator. The in-code module doc in `src/supervisor.rs`
and all owned docs were updated.
