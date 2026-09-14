# T9b — adversarial review gate, round 2 (post-T9 code: F14/F16/F18 + the four rulings)

Reviewer: T9b agent, read-only. Branch `review/t9b`, head `7140198`. Method: read DESIGN.md (binding),
PRD.md, orchestration/LEDGER.md; read the full post-T9 diff (`8c6d2fd..HEAD`: supervisor.rs +457,
smc.rs +159, cli.rs +129, tests +276, policy.rs +10); ran all four §8 gates; then attacked the
mandated defect class with three **live probes** — the real binary against tempdir copies of
`tests/fixtures/sysfs/` (never the repo fixture, never real `/sys`, no `--features hw` runs). No
source files were modified.

## Gate evidence (this checkout, this turn)

- `cargo fmt --check` — pass.
- `cargo clippy --all-targets --all-features -- -D warnings` — pass.
- `cargo test` — pass (131 tests incl. integration + schema).
- `cargo test --features hw` — all suites pass; the hw guard test skips cleanly without applesmc.

## Verdict: FAIL — two MAJOR holes in the post-F16 safety envelope, both reproduced live

The three tasks (F14, F16, F18) match their frozen rulings, and the ruling tests genuinely pin the
rulings (kill-analysis below). But the mandate's central hunt — "with F16's split, what can now go
unnoticed that the old (buggy) code would have caught" — found two real holes, and both were
reproduced with the shipped binary against fixture copies. The old (tach-counting) code caught both;
the new code does not.

---

## Findings (prioritized)

### 1. MAJOR (proven live) · `src/supervisor.rs:593-608` — a changing command resets the stall window every poll; a dead actuator is invisible while temps oscillate mid-band

**What is wrong.** The stall window (`stall_cmd`/`stall_polls`/`stall_base_dev`) resets whenever the
written target changes (`if self.stall_cmd != Some(written) { … reset … }`, supervisor.rs:594-598).
F16's L1 split makes tracking deviations explicitly uncounted (supervisor.rs:573-592) — deliberate
and correct — so the stall detector is the *only* net left for "commanded but never moving". But
during curve slew the target legitimately changes every poll, and mid-band t_eff oscillation keeps
it changing indefinitely. In that regime the window never reaches 10 polls and a completely dead
actuator is never declared.

**Why it matters.** Verified-by-echo writes keep succeeding (the register takes every new target),
so the plugin sees `verified: true`, `monitor_only: false`, empty `recent_errors` — a *healthy* render
for a machine with a dead fan at 80–84 °C. The old buggy code counted the >150 rpm deviation and
degraded after 3 polls. Worse: the tracking re-assert notes go to `StepReport` only and `run()`
discards the report (supervisor.rs:240-243), so there is zero journal evidence either.

**Reproduction (proven this turn, real binary, tempdir fixture copy; never touched real /sys):**
fixture copy with `fan1_input` pinned at 1200 (dead actuator), `temp*_input` alternated 80/84 °C
every 0.4 s (sustained mid-band oscillation), `daemon --mode curve`:
after 38 polls → `verified: true`, `monitor_only: false`, `recent_errors: []`, zero `stall detector`
lines in the journal. (`once` the oscillation stops, the command stabilizes and the stall fires
~10 polls later — a second probe confirmed the detector works once the command is constant.)

**Suggested fix direction.** Key the window on the *fan's failure to approach a moving target*
rather than on target-change events: e.g. reset the window only on observed fan movement toward the
(current) command, or maintain "polls since the fan last moved toward the command at all"; a
command change must not buy a dead actuator a fresh 10-poll lease. Keep N-F16-1's direct-degrade.
Add a regression test in the `f16_*` family with sustained target motion + frozen tach.

### 2. MAJOR (proven live) · `src/supervisor.rs:655-680` + `561-565` — fallback with a failed AUTO restore leaves the fan in Manual with L1 permanently disabled and no AUTO re-attempt

**What is wrong.** `degrade_to_auto` sets `manual_armed = false` (supervisor.rs:667) even when its
own `set_mode(FanMode::Auto)` just failed (supervisor.rs:658-665). Every subsequent `l1_poll`
early-returns ("not ours to verify", supervisor.rs:561-565), so mode-drift and stall checks are off,
and `act` writes nothing (`monitor_only`, supervisor.rs:470-471). Nothing ever re-attempts AUTO:
`ReturnToAuto` is unreachable in `monitor_only`, the cmd-file freshness gate suppresses re-arming
(supervisor.rs:355-357), and only a *changed* command re-arms (supervisor.rs:414-423). The in-code
comment ("L2/L3 remain the final nets", supervisor.rs:660-661) is wrong for this state: the process
is alive and watchdog-pinging, so L3 never fires, and L2 fires only on death.

**Why it matters.** Reachable chain: 3 consecutive verified-write failures while
`fan1_manual=1` (echo mismatch — *observed on real hardware in the F16 event*) plus a `set_mode(Auto)`
verify failure (e.g. a dueling controller or a wedged SMC re-asserting `fan1_manual=1` within the
verify window) → the fan is stranded in Manual at the last written speed, unsupervised, indefinitely.
The F18 latch is visible (`monitor_only: true` in state/status) — the half F18 fixed — but the fan
itself is stranded until a *changed* command arrives or the process dies. A plugin that honors
`monitor_only` by going quiet strands the fan forever.

**Reproduction (proven live, probe A):** fixture copy, `fan1_manual` chmod 0444, `hold 4000` in
`cmd.json`, `daemon` (observe, no `--mode`): journal shows exactly
`fallback: set AUTO itself failed; monitor-only without AUTO restore` (the branch at
supervisor.rs:658-665 firing on real code), then total silence — no further writes, no L1 activity,
state frozen at `monitor_only: true`. (In this probe the fan file was 0 so no physical stranding
occurred, but the supervisor state machine reached and rested in the hole.)

**Suggested fix direction.** On failed AUTO restore in `degrade_to_auto`, schedule a periodic
best-effort AUTO re-attempt even while latched (e.g. one `set_mode(Auto)` every N polls while
`monitor_only`), and/or keep the L1 read alive in the degraded state so the plugin/journal sees
`mode != Auto` drift rather than `verified: true`-style silence. Do not weaken N-F16-1's
direct-degrade.

### 3. MINOR (proven live; logic hole predates T9 but F14 rewired `run()` around it) · `src/supervisor.rs:222-227` vs `414-423` — the "no L2 → observe" startup guard is bypassed by the cmd file

**What is wrong.** `run()` degrades the startup mode to observe when `panic_fd()` is `None`, with the
explicit claim "no control without a trustworthy safety net". `apply_mode` re-arms control from
`cmd.json` without ever checking `panic_fd()`.

**Reproduction (proven live):** fixture copy, `fan1_manual` 0444, `hold 4000` cmd present, `daemon`
(observe): startup evidence line prints `l2_armed=false`, then the first poll commands `hold` and
attempts three `fan1_manual` writes before the fallback latches. Control was entered with no L2.

**Reachability honesty.** On the real backend `panic_fd() == None` implies `fan1_manual` was not
O_WRONLY-openable, which also makes every later write fail (probe A confirmed: 0 physical writes).
Real-hardware reachability is therefore narrow (a pre-open failure that later permissions fix, or a
mid-life permission change); the installed unit runs as root where O_WRONLY always succeeds. So: a
proven hole in a *stated safety invariant*, low live-hazard. Still worth closing — the probe shows
the guard is one line short of its own promise.

**Suggested fix direction.** Wire the same `panic_fd().is_none()` check into `apply_mode` for
writing modes (hold/curve), so the guard holds regardless of which channel commands control.

### 4. MINOR (code-derived, not live-repro'd) · `src/supervisor.rs:604-608` — a fan hovering just outside tolerance is invisible forever

A deviation oscillating just above `VERIFY_TOLERANCE_RPM = 150` (real tach jitter near the boundary)
trips `dev < self.stall_base_dev` on every dip, resetting base and polls; tracking re-asserts are
uncounted by ruling. The fan can sit permanently ~150–250 rpm off target with `verified: true` and
(no) stall, in a slow re-assert loop. The deterministic mock (`tach_slew`, smc.rs:519-526 — strictly
monotonic, zero jitter) cannot express this; the *old* code counted it. Low hazard (fan is spinning,
near target), but it is the answer to the mandate's "slow drift" sub-question.

**Suggested fix direction.** Count "time since last convergence" separately from the
unresponsive-actuator window (e.g. an off-target dwell counter that is insensitive to jitter), or
widen the stall window's reset criterion to require movement toward the command, not just a lower
deviation.

### 5. MINOR (contract drift) · `DESIGN.md:97-101` vs `src/policy.rs:42-46` — Appendix A's constants block was not updated for the two ruled constants

`WRITE_ECHO_TOLERANCE_RPM = 50` and `STALL_POLLS = 10` are ruled (F16) and described in the
Appendix-A trait doc comment (DESIGN.md:59-71), but the binding constants list at DESIGN.md §7
still lists only the pre-F16 five. Appendix A is the binding contract; the two new public constants
should appear there with their frozen values.

### 6. MINOR (test values unpinned) · `src/policy.rs:42,46` — no test would notice `50` → `150` or reveal the tolerance value at all

Echo reads are exact on fixtures and in `MockSmc` (instant mode), so no test discriminates the 50
value (`mock_verify_accepts_within_tolerance_readback`, smc.rs:759-767, passes at any tolerance
≥ 50), and `f16_frozen_fan_stall_detector_falls_back` loops `0..STALL_POLLS`
(supervisor.rs:1436) — it pins "fires within STALL_POLLS polls", not the value 10. A change to
either constant would pass the entire suite. The values are defensible against real firmware (echo
is a register, not physics), but the ruling froze numbers the tests don't pin.

### 7. MINOR (docs) · `README.md:14-18` — the L1 safety-model paragraph still reads as pre-F16 semantics

"After 3 consecutive failed re-assertions it restores AUTO" sits under a sentence listing the >150 rpm
deviation as a trigger; post-F16 a tracking re-assert is *never counted* (a tracking re-assert counts
only when it itself fails, supervisor.rs:588-590). A reader can wrongly infer rpm mismatches feed the
3-strike ladder (the exact pre-T9-F16 misconception). Reword to the split semantics; the plugin
section (README.md:181-187) is accurate.

### 8. MINOR (untested surface) · `src/supervisor.rs:315-327` — the startup evidence line, including the F18 `config_source` field, has no test

No test asserts the evidence line's fields; the journal is only ever asserted for the reconcile
message (tests/integration.rs:564-568). `RuntimePaths.config_source` is consumed by exactly one log
field (supervisor.rs:323); it could silently regress. (The ledger's F18 verification was a source
read, not a test.)

### 9. MINOR (documented-behavior gap) · `src/supervisor.rs:274-296` — startup reconcile takes the fan from any foreign Manual owner

Reconcile cannot distinguish "our SIGKILLed predecessor" from "a live foreign program holding manual"
(e.g. mbpfan): every afanctl (re)start restores AUTO out from under it. Ruling F14-compliant (the
ruling is unconditional) and philosophically consistent with "fail toward the firmware", but the
interaction is undocumented — worth one README sentence, not a code change.

## Explicitly checked and clean

- **RULING F14** — compliant. `run()` order is arm L2 → `reconcile_stale_state()` → evidence line →
  sd_status → sd_ready → loop (supervisor.rs:221-244); Manual ⇒ one verified `set_mode(Auto)`; failed
  restore / failed read while writing ⇒ observe + `monitor_only` (supervisor.rs:255-309); Auto ⇒ zero
  writes; evidence line names mode/L2/notify/state/config_source/hw-band (N-F14-1 as approved in
  F18/A4). Reconcile runs before READY and after L2 arming (panic during reconcile → L2 writes AUTO).
  Kill-analysis: deleting reconcile fails `daemon_observe_reconciles_stale_manual_state`;
  unconditional AUTO write fails `reconcile_auto_writes_nothing`; double-write fails
  `reconcile_manual_restores_auto_with_a_single_verified_write`. Reconcile + pre-existing `hold` cmd:
  reconcile restores AUTO, first poll re-applies the command and re-enters control — consistent (the
  cmd file is the standing plugin command; `last_cmd` starts empty).
- **RULING F16** — compliant. `SysfsSmc::write_speed` verifies against the `fan1_output` echo within
  50 (smc.rs:340-369; `fan1_input` never verified against, smc.rs doc 11-14); L1 split exactly per
  PRD R4/line 127 (mode-drift counted, supervisor.rs:567-572; tracking uncounted, 573-592; stall at
  `STALL_POLLS` degrades directly per N-F16-1, 609-623); `MockSmc` gains lag/frozen/write-stuck
  (smc.rs:494-504) and the Auto-mirror rule (466-472, 556-561); the F16-defect-encoded fixture test
  was replaced by `fixture_write_verified_by_register_echo` (smc.rs:770-782) per N-F16-2. Kill-analysis:
  reverting the echo source fails `fixture_write_verified_by_register_echo`; counting tracking fails
  `l1_tracking_deviation_reasserts_without_counting` / breaks `f16_hardware_event_converges_without_fallback`;
  removing the stall detector fails `f16_frozen_fan_stall_detector_falls_back`; counting tracking as
  mode-failure fails `l1_tolerance_band_suppresses_reassert`. (Residual gaps are findings 1, 4, 6.)
- **RULING F18** — compliant and genuinely additive. `monitor_only` in `state.v1` (supervisor.rs:119,
  published 710) and `status.v1` (cli.rs:725), schema ids unchanged, both Appendix-B examples updated
  (DESIGN.md:183,198); human status renders mode (+`(monitor-only)`), target, recent errors
  (cli.rs:751-823 — the F17 items); smc pre-open failure demoted to `tracing::debug!` (smc.rs:287);
  `config_source` threaded cli → RuntimePaths → evidence line (cli.rs:355-388, supervisor.rs:54-61,
  323). A v1 consumer ignoring `monitor_only` keeps working. Kill-analysis for the latch tests:
  removing the field fails `fallback_after_write_failures_degrades_to_monitor_only`
  (supervisor.rs:1165-1168) and the F18 integration regression; dropping the human marker fails
  `curve_write_failure_degrades_and_status_marks_monitor_only`.
- **Contract hygiene**: no `unsafe` outside `safety.rs` (grep + compile); zero
  `unwrap`/`expect`/`panic!` in product code (grep over pre-test regions + clippy `-D warnings`
  green); dependencies exactly the R11 allowlist; Appendix A signatures unchanged except the ruled
  `RuntimePaths{+config_source}`; no `println!` outside `cli.rs`; module docs present. LOC budgets
  remain massively over §7 (cli.rs 1385 / supervisor.rs 1503 / smc.rs 978 / doctor.rs 1063 lines incl.
  tests) — standing accepted deviation, stop-and-reported in the ledger pre-T9 with drivers named;
  F16 (+155 / +250) and F18 (+79 / +150) product deltas stayed within their ticket budgets.
- **Docs vs README/human status**: `status` human output delivers every field the README table
  promises (mode/target/fan band/manual/provenance/recent errors); `hold` reject-below-min/clamp-max
  matches (cli.rs:609-625); exit codes match `-h`.

## Assumptions that only hold on fixtures

Proven section vs speculative section kept separate; hardware evidence cited where it exists.
(Constrained by the card: no hardware tests were run by this review.)

**Evidence-backed on real hardware (gate (b)/(c), ledger):** echo-verify works for one point —
`fan1_manual` mode flip + `fan1_output` = 1200 in Manual, exact echo, sync manifest. Everything
beyond that point is inference below.

1. **`fan1_output` in Auto mirrors the SMC's own target** (mock rule, smc.rs:466-472, 556-561).
   *Speculative.* The F16 journal read-backs (6170/5235/1866) were `fan1_input` values read by the
   old tach-verify code — there is no real-hardware evidence at all of what `fan1_output` returns in
   Auto. Not load-bearing for product code (writes only happen after a verified `enter_manual`), but
   the mock's "Auto takeover" realism rests on it.
2. **Register echo is exact and instantaneous within 50 rpm of any in-band target.** Gate (b)
   evidenced exactly one target (1200). *Speculative:* unknown firmware quantization/rounding at
   other targets. Fail-safe direction holds (echo mismatch → K=3 → counted → AUTO + monitor-only),
   but if quantization exceeded 50 rpm, curve mode would degrade at every write — the F16 class
   reborn at the register level. The re-run of supervised gate P5 (f) post-F16 is the missing
   evidence; the fix tolerance value is also test-unpinned (finding 6).
3. **`fan1_manual` manifests synchronously** (read-back immediately after write, K=3 back-to-back
   with no delay, smc.rs:377-395). Gate (b) shows it holds on this machine at 0→1→0. *Speculative*
   under SMC load/other states; zero inter-attempt delay means three attempts span microseconds.
4. **`read_fan` treats two separate file reads as one atomic snapshot** (smc.rs:321-327). *Speculative
   and benign:* a torn read (rpm from t₁, mode from t₂) self-corrects next poll.
5. **`fan1_input` is instaneous rpm with jitter ≪ 150 rpm.** Gate (b) observed ~10 rpm. *Speculative*
   generally; interacts with finding 4.
6. **Test-only: `MockSmc` tach physics are deterministic and monotonic** (`tach_slew`, fixed rate,
   no noise) — the stall window's convergence-reset behaves differently under real jitter; that is
   precisely how findings 1 and 4 escape both the mock and the static fixtures.

## Provenance notes

- Probes A/B/C ran only against `/tmp/opencode` tempdir copies of the repo fixture; the repo fixture
  and real `/sys` were never touched; the running system daemon (installed unit from the hardware
  gate) was observed read-only and not interfered with. All probe processes and files were removed.
- Unproven/supposition has been labelled as such above; nothing on the list is padded.
