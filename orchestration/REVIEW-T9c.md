# T9c — adversarial review gate, round 3 (F19 + F20, with an attempt to falsify RULING F20 R1)

Reviewer: T9c agent, read-only. Branch `review/t9c`, head `3ddb929`. Method: read DESIGN.md
(binding), PRD.md, orchestration/LEDGER.md, orchestration/REVIEW-T9b.md; read the full post-T9b diff
(`b0fba6e..HEAD`: supervisor.rs +555, smc.rs +340, policy.rs +30, doctor.rs +136, cli.rs +12,
tests +317, docs); ran all four §8 gates; then attacked the mandated classes with **live probes**
against tempdir copies of `tests/fixtures/sysfs/` (never the repo fixture, never real `/sys`, no
`--features hw` actuation runs). Four read-only hardware observations were made on this machine
(applesmc present): tach sampling, `doctor` (no `--roundtrip`), `systemctl show`, journal/state
reads of the installed daemon. No source files were modified.

## Gate evidence (this checkout, this turn)

- `cargo fmt --check` — pass.
- `cargo clippy --all-targets --all-features -- -D warnings` — pass (unwrap/expect/panic lints
  enforced at crate level via `[lints.clippy]`, Cargo.toml).
- `cargo test` — pass: **152 tests** (126 unit + 11 integration + 7 traces + 4 schema + 2
  settle_window + 2 pin_constants).
- `cargo test --features hw` — all suites pass; `hw_guard_skips_without_hardware` skips cleanly.
  Note: **this box has a real applesmc** and the installed daemon is `active` — `AFANCTL_HWTEST`
  unset therefore exercised the guard for real, and no hardware writes occurred from the gate.

---

## Verdict: FAIL — one MAJOR (new, proven live) + one mandated-cost finding + 5 MINOR

F19 and F20 each match their frozen rulings, and the ruling tests genuinely pin the rulings
(kill-analysis in the "Regression risk" section). But mandate 3's hunt for "honest AUTO-restore"
found a sibling path F20 R2 did not cover, reproduced live below, and mandate 2's cost question
produced a concrete arithmetic answer that invalidates one corner of the config range.

### 1. MAJOR (proven live, probe B) · `src/supervisor.rs:486-490` — the cmd-file observe transition drops ownership when its own AUTO restore fails: the T9b MAJOR-2 class resurfaces on the cmd-file channel

**What is wrong.** `apply_mode`'s observe transition (`requested == Observe && mode != Observe`) on
`set_mode(FanMode::Auto)` error does `self.manual_armed = false; … self.fail_write(…)` — it drops
ownership of the fan on a *failed* release. `degrade_to_auto`'s Err branch was fixed by RULING F20
R2 exactly because of this shape (kept `manual_armed`, armed `auto_restore_pending` — the
`SAFETY-INVARIANT` comment at supervisor.rs:762-764), but the observe-cmd branch got no equivalent:
no `auto_restore_pending`, no every-poll retry, and `monitor_only` stays false unless this happens
to be the 3rd strike (`write_failures` was 0 before; it lands at 1 and never advances because the
observe supervisor writes nothing). `act()`'s `ReturnToAuto` Err branch (supervisor.rs:536-538)
correctly does *not* clear `manual_armed` — the observe branch is the one inconsistent path.

**Why it matters.** After the failed release the fan is still `fan1_manual = 1` at the last written
speed; `manual_armed == false` makes every later `l1_poll` early-return "not ours to verify"
(supervisor.rs:621-626) — mode-drift, tracking, stall and dwell checks all off; nothing writes;
nothing retries AUTO; `last_cmd` was updated so the freshness gate suppresses re-apply of the same
observe command (supervisor.rs:394). State renders fully healthy, differing from T9b MAJOR-2 only
in that the fan is parked in Manual at the previous speed rather than stranded with no recall at
all.

**Reproduction (proven this turn, real binary, tempdir fixture copy; real `/sys` untouched).**
Fixture copy: `daemon` with `cmd.json = hold 3000` → after ~3.5 s `fan1_manual=1`,
`fan1_output=3000` (we own the fan). Then `fan1_manual` made unwritable (EACCES — one
deterministic stand-in for the dueling-controller verify failure the ruling itself names) and
`cmd.json = observe`. Result: journal `cmd observe: set AUTO error: write …: Permission denied
(os error 13) (fallback 1/3)`, then `state.json` = `{mode: "observe", verified: true,
monitor_only: false, auto_restore_pending: false, …}` with a single rotating `recent_errors`
entry, **while the fixture held `fan1_manual=1 / fan1_output=3000`** (confirmed mid-strand;
after SIGKILL the file still read `manual=1`). No `AUTO restored`/retry line ever. A second variant
of the same trigger — a live second writer re-asserting `fan1_manual=1` inside the
`MODE_SETTLE_MS` window — lands on the identical branch (it is the exact reachability premise T9b
MAJOR-2 used).

**Suggested direction.** Mirror R2 in the observe branch: on `Err`, keep `manual_armed`, set
`auto_restore_pending` (the existing `auto_restore_retry` then keeps the release honest even
though mode is now observe), and do not claim the mode applied; optionally `degrade_to_auto`-style
latch. One branch, ~6 lines; same fix class as the ruled 658-665 correction.

### 2. MINOR (arithmetic-proven, not live-systemd-repro'd) · `src/policy.rs:47,50,53` + `src/config.rs:41` — the settle window consumed the watchdog margin that `MAX_INTERVAL_S = 14` was calibrated against; `interval_s = 14` can now SIGABRT-crash-loop a *healthy* curve daemon

`settle()` sleeps `window_ms / ECHO_SETTLE_SAMPLES` between 10 samples (smc.rs:151), so a failing
echo window costs 9 × 150 ms ≈ 1.35 s, doubled by `WRITE_RETRY_MAX = 1` → **≈ 2.7 s per failing
`write_speed`, ≈ 1.8 s per failing `set_mode`**. Measured independently by the suite's own
wall clock: `tests/settle_window.rs` test 2 (3 failing-echo polls) finishes in **8.11 s**.
Consequences, per ping-gap = interval + poll work (ping at step_once end, supervisor.rs:228-230):

- `interval_s = 1` (default): worst-case poll ≈ 2.7 (act) + 2.7 (L1 re-assert) + 1.8 (L1 mode
  re-assert) ≈ 7.2 s → max ping gap ≈ 8.2 s < 15 s. **Safe, ~45% margin.** No defect in the
  default shape.
- `interval_s = 14` (legal: `1..=14`, `MAX_INTERVAL_S = WATCHDOG_UNIT_SEC − 1`): a *healthy*
  curve poll with a changed target blocks for one accept-window ≈ 0.15–1.05 s (hardware-measured
  adoption ≤ 1 s, ledger 20:47) → ping gap up to ≈ 15.05–15.5 s **≥ `WatchdogSec=15`** →
  intermittent SIGABRT → L2 → restart, exactly during sustained temp oscillation (the regime
  curve mode exists for). A failing-echo regime is worse: gap ≈ 14 + 2.7 = 16.7 s per poll.
  Pre-F19 the same corner cost ~0 (µs retries), so F19 silently invalidated a value the config
  validation still blesses; `doctor`'s config check even warns it "must stay below the unit's
  WatchdogSec=15" (README) — at 14 with the settle cost that is no longer true.

Fail-safe direction holds (L2 restores AUTO on the SIGABRT; unit restarts), so this is a
crash-loop nuisance, not a fan-safety hole — but a legal config intermittently kills the daemon
instead of degrading. **Direction:** ping twice per poll (start and end of `step_once`) or lower
`MAX_INTERVAL_S` (≤ 12 keeps ≥ 3 s margin under the 1.5 s + re-issue worst case). One-constant/
one-call fix; needs a ruling because the cap is bound to the unit.

### 3. MINOR (proven live, probe A; real-hazard magnitude **unproven**) · RULING F20 R1's jitter residual + R4's once-per-lifetime warn — a permanently off-target, moving-when-it-jitters fan degrades **never** and goes silent after exactly one WARN

Probe A (49 polls, `hold 3000`, `fan1_input` alternated 2760/2700 — 60 rpm > `STALL_TACH_EPSILON_RPM`
each poll, parked 270–300 rpm off target): `monitor_only:false`, `verified:true`, **zero** stall
lines ever, exactly **one** `off target for 30 polls (moving, not stalled)` WARN at poll 30
(supervisor.rs:663-674), then silence for the remaining life of the daemon. This is not
accidental: `f20_off_target_dwell_warns_once_per_excursion` (supervisor.rs:1837-1891) *encodes*
jitter 60 > 50 as perpetual progress — the shipped test is the falsification. The stall criterion
counts any per-poll delta > 50 as progress (supervisor.rs:678-683), so a fan jittering ±60–100 rpm
while unable to follow its command (bearing degradation, partial obstruction — the mandate's
hypothetical) is never declared. Full analysis and the per-case verdicts are in the dedicated
RULING F20 R1 section below.

### 4. MINOR (contract drift) · `DESIGN.md:108-114` vs `src/policy.rs:50,56` — Appendix A's binding constants list omits two ruled public constants

`ECHO_SETTLE_SAMPLES = 10` and `WRITE_RETRY_MAX = 1` are public product constants frozen by
RULING F19 (policy.rs:50,56; the trait doc prose mentions "10 × 150 ms" and "re-issued once") but
the binding constants block lists neither. Same defect class as T9b finding 5, reintroduced by the
next ruling round. Two lines in DESIGN.md §Appendix A.

### 5. MINOR (values unpinned, T9b finding 6 class) · the F19 constants and three F20 constants have no value-pin

`tests/pin_constants.rs` pins `WRITE_ECHO_TOLERANCE_RPM` at the ±1 boundary and `STALL_POLLS`
exact-fire — genuinely pinned. Not pinned, with the widest gap an editor could slip through:

- `ECHO_SETTLE_MS` / `MODE_SETTLE_MS` / `ECHO_SETTLE_SAMPLES` / `WRITE_RETRY_MAX`: every F19 test
  uses *relative* values (300 ms injected latency; `2 × ECHO_SETTLE_MS`). The suite constrains the
  window to roughly (300 ms, 3 s) — `ECHO_SETTLE_MS = 400` or `= 2000` passes everything.
- `STALL_TACH_EPSILON_RPM = 50`: probe A2 shows 15-rpm jitter still stalls; the shipped tests
  require only ε < 60 (test 6's 2760/2700 alternation) and ε < 1500 (test 2's moving fan), so
  ε = 55 passes the entire suite — the real-jitter margin (measured: see below) is pinned by
  nothing.
- `AUTO_RETRY_LOG_POLLS = 10`: the rate-limit has no test at all (the "first log, then every 10"
  shape could silently become "log every poll").
- `OFF_TARGET_WARN_POLLS = 30`: pinned only as "fires exactly at the constant" (pin_constants.rs
  and supervisor.rs test both loop the symbol); any value passes.

### 6. MINOR (docs) · `README.md:21-22` — the L1 sentence overpromises by exactly the probe A case

"A fan whose tach never moves **toward its command** is caught by a separate stall detector" —
the detector catches fans whose tach never moves *at all* (supervisor.rs:678-683 keys on any
movement > 50). A fan that keeps moving but never toward the command is not caught; it gets one
WARN (finding 3). One clause: "never moves toward" → "never moves between polls".

### 7. MINOR (test cadence gaps surfaced by kill-analysis) · details in the Regression-risk section

Retry-genuinely-every-poll (R2) is only weakly pinned (an every-2nd-poll retry passes
`f20_failed_auto_restore_is_retried_truthfully` unchanged); the once-per-excursion warn latch is
unpinned (a repeating-warn semantics change passes the whole suite — which matters because that
repeating warn is the proposed direction for finding 3 and nothing would pin it).

---

## RULING F20 R1: **survives — with two proven residuals and one unproven hazard** (recommendation: keep as ruled, amend R4's warn to repeat)

The criterion as implemented (supervisor.rs:678-683): an armed, off-target poll (dev > 150) counts
as progress when `|tach_now − tach_prev| > STALL_TACH_EPSILON_RPM (50)`; motionless polls
accumulate; `STALL_POLLS (10)` → direct degrade. Four attack cases, concretely:

**(a) Fan jittering ±60–100 rpm while parked far from the command — the detector never fires;
jitter counts as progress forever. PROVEN (probe A, above; and encoded by the repo's own test 6).**
±60 rpm jitter is strictly "progress" every poll → the window never fills → the only net is R4's
dwell WARN, which fires **once per excursion** (`off_target_warned` latches until the fan
converges or degrades, supervisor.rs:665-674, 698-704) — for a permanent condition, one line
forever. *Unproven:* whether real fans in Manual jitter that hard. Read-only measurement this
turn: real `fan1_input` at ~3100 rpm (SMC-owned, Auto), 20 samples @ 1 Hz → per-second deltas
0–26 rpm, max 26 — i.e. the observed worst case is ~half of epsilon 50, and the mandate's
±60–100 scenario would be 2–4× anything measured here. Finding 3 is the visibility fix; the
stall/false-quiet trade itself is defensible as ruled.

**(b) SMC clamps the command; tach stable; detector fires; degrades — false positive? Largely
unreachable through the stall detector; where reachable, degrading is the right answer.** Three
routes, all traced from code:
- Clamp within `WRITE_ECHO_TOLERANCE_RPM (50)` of the target → write verifies; tach settles at
  the clamped value ≤ 150 from command → inside `VERIFY_TOLERANCE_RPM`: nothing fires at all
  (benign; ≤ 50 rpm effective error).
- Clamp beyond 50 (echo never holds the value) → `VerifyFailed` → the 3-strike ladder, *not* the
  stall detector (`last_written` never commits a failed write) → AUTO + monitor-only after 3.
- The only sub-path that reaches the stall detector: echo verifies (≤ 50) but the physical fan
  cannot follow (true floor > commanded + 150, tach motionless) → fires at 10 polls → AUTO +
  monitor-only. That requires `fan1_min` to under-report the real floor, contradicted by
  measured hardware on this machine (gate (b): command 1200 → observed 1210; 2000 → converged
  ~2000 ± 20; today's live sample: fan at 3077–3183 with `fan1_min=1200` respected). And even
  then, degrading is correct-by-design: the fan demonstrably cannot execute the command, so
  fail-toward-firmware is the honest answer; it re-triggers on each re-command (cn00l loop of
  10-poll windows), which is noisy but truthful. **No falsification here, unproven premise.**

**(c) Fan moving away while the command ramps faster than the fan can follow — a healthy fan is
never falsely declared. PROVEN SAFE under measured physics.** Command slew = 750 rpm/poll
(policy.rs:32) vs fan ~2000–3000 rpm/s (ledger 20:47): the fan outruns the command; while
off-target its delta is 10–100× the epsilon, so every poll resets the window and convergence ends
inside tolerance (test f16_hardware_event, tests f20_sustained_motion). The only arithmetic shape
that false-fires — **net convergence < 15 rpm/poll for > 10 consecutive polls while > 150
off-target** — needs a fan ~10× slower than measured; and the "moving *away*" variant requires
the fan to slew *against* the register it was just verified to hold, which a healthy fan does not
do (mirrors are an Auto-mode behavior; in Manual the register is our value).

**(d) Frozen tach under a moving command (the original T9b MAJOR-1) — fires. PROVEN** by
`f20_moving_command_frozen_tach_stall_fires` (probe-equivalent shipped test, latches ≤
STALL_POLLS + 3 under alternating targets) and `f20_stall_fires_exactly_at_stall_polls`.

**Why "movement at all" is still the right rule, and the minimal amendment.** Both superficially
better criteria fail cases: *movement toward the command* (`dev < prev_dev`) is defeated by
symmetric jitter around a parked point (2700→2760 halves the deviation on alternate polls);
*net-deviation-over-the-window* risks a false stall on a healthy fan chasing a ping-ponging
command, whose 10-poll-window net deviation is a coin flip. Per-poll classification cannot
separate "jittering but never converging" from "healthily chasing without converging yet".
**Recommendation: keep R1 as ruled** for degradation, and close case (a) in the R4 channel —
make the off-target dwell WARN repeat every `OFF_TARGET_WARN_POLLS` while an excursion persists
(drop the permanent `off_target_warned` latch, supervisor.rs:666 — re-warn each 30 polls ≈ one
line/30 s at default cadence), or escalate WARN→ERROR after the first excursion completes. That
converts a silent-forever condition into a periodically visible one with **zero**
false-positive risk, and pins nothing new in the stall semantics. If the orchestrator prefers a
criterion change, the only candidate that survives (a) is a long-horizon net-deviation
check (e.g. low-water deviation improvement over 3 × window with hysteresis) — more machinery,
real false-positive risk during ramps, and no hardware data justifying it; not recommended
without gate (f) re-measurement.

---

## F19's settle window — false-PASS attack (mandate 2): **no material false PASS; the cost answer is finding 2**

**Is there a target for which the check reports success while the fan is not actually commanded
there?** No material one. Traced through `settle_write` (smc.rs:169-211) and both backends:
- The write syscall precedes the window's first sample (smc.rs:181-189), and a write-syscall
  error aborts immediately (R4) — accepted-but-unverified writes are the only thing a window
  ever settles.
- **Stale-echo instant match**: when the register already holds the target (or the previous
  value within 50 of the new target — e.g. slew deltas), the t=0 sample matches instantly and
  no adoption proof is collected. But "the register holds the target" *is* the command being in
  place (`F0Tg` is the command register; a dropped re-write that the register already reflects
  commands exactly the same speed), and any stale-vs-new mismatch that passes the window is
  bounded by `WRITE_ECHO_TOLERANCE_RPM = 50`. The fan may be commanded ≤ 50 rpm off the nominal
  target — inside `VERIFY_TOLERANCE_RPM` (150), invisible and inconsequential (the SMC
  quantization/clamp class folds into the same ≤ 50 bound). A clamp *greater* than 50 off never
  passes; it fails the window and travels the counted-failure ladder instead.
- **Mode channel:** exact-match on a bit (smc.rs:429-448). A stale match means the register
  already reads the wanted mode — which is success. The one real false-PASS risk is the
  **competition window**: a dueling controller that flips `fan1_manual` inside the 1 s window is
  undetectable the moment the register re-flips *and comes back* — but that also requires the
  register to read `Auto` for L1's next-poll drift check to catch re-assert duty, and the probe B
  branch shows the observe transition is exactly where the consequences go wrong (finding 1).
- `write_speed` returns the *commanded* value, not the read-back (smc.rs returns `Ok(wrote)`), so
  `last_written` may sit ≤ 50 rpm off the true register value — again inside L1's 150 band.
  Consistent with Appendix A's "Returns the verified rpm" (the verified-in-band value), worth a
  doc comma but no defect.

**What does the window cost?** Answered concretely as finding 2 (2.7 s / 1.8 s per failing call;
≈ 7.2 s worst poll at default cadence = safe vs `WatchdogSec=15`; ≈ 15.0–16.7 s gap at
`interval_s = 14` = not safe). Additional bounded costs, for the mandate's ledger: the
overshoot guard's 3-poll escalation stretches to ≈ 3 × (1 s window + 1 s sleep) ≈ 6 s on a
changed target — acceptable against thermal slew; cmd-file latency under the plugin's
re-command story becomes ≈ poll period + the in-flight poll's blocking (≈ 2–4 s in a
failing-echo regime, ≈ 2 s in healthy curve slew) — the freshness gate means the fresh bytes are
read at the *top* of the next poll, so the extra latency is at most one long poll.

## AUTO-restore recovery, F20 R2 (mandate 3): **compliant; no unverified-success path; no silent latch**

- **Every-poll retry is genuine:** `auto_restore_retry` is called unconditionally at
  supervisor.rs:205, before any control decision, every poll while `auto_restore_pending`.
- **No success without verification:** success path is either `read_fan() == Auto` (a direct
  register read-back) or `set_mode(Auto) == Ok` (which verifies internally by exact read-back,
  smc.rs:431-448). No code path flips the flag on an unverified write.
- **No latched-forever-while-healthy:** if the retry keeps failing, `auto_restore_pending` stays
  `true` in state.json (publish_state, supervisor.rs:852) and status JSON (cli.rs:732, and human
  marker cli.rs:777-778 `auto_restore_pending: true (AUTO restore pending)`) every poll, with a
  rate-limited ERROR every `AUTO_RETRY_LOG_POLLS` polls — the *flag is the render*, so it cannot
  hide. The one cosmetic wrinkle: `apply_mode`'s successful observe release does not clear the
  flag; the next poll's retry sees the register Auto and clears it (one-poll self-heal,
  supervisor.rs:785-821) — harmless.
- **Re-arm after a late successful restore:** intentionally *none* — `monitor_only` stays
  latched, the controller runs `step_observe`, and control re-arms only via a changed command /
  restart (F10's deliberate latch, README "To re-arm after monitor-only degradation…"). State
  reads `auto_restore_pending:false, monitor_only:true` — accurate.
- The gap is finding 1: the *observe-cmd channel* bypasses the exact machinery this
  verdict covers.

## Regression risk (mandate 5): kill-analysis, F19/F20

**F19 (settle window) revert-killers:** `mock_write_not_taking_fails_after_window_retries`
(smc.rs:861-871 — asserts exactly `WRITE_RETRY_MAX + 1` attempts, dies under the pre-F19 3-attempt
ladder); `echo_latency_is_accepted_not_a_failure`; `set_mode_latency_verifies_within_window`;
`tests/settle_window.rs` test 1 (300 ms latency fails instantly under µs retries). Pass-either-way
weak spots: `latency_beyond_window_fails_verify` and settle_window test 2 (genuine-failure cases),
all `f16_*` supervisor tests (they run the mock with `echo_latency: None` — instant register
adoption, so pre-F19 semantics are invisible to them · finding 4 of "mock-shaped assumptions").

**F20 revert-killers (all rulings hold):** R1 → `f20_moving_command_frozen_tach_stall_fires`
dies; R2 → `f20_failed_auto_restore_is_retried_truthfully` dies (incl. the no-success-lie and
"after N attempts" asserts); R3 → `f20_l2_absent_refuses_cmd_control` dies; R4 →
`f20_off_target_dwell_warns_once_per_excursion` dies. R5's pinning exists (pin_constants.rs) and
the evidence-line test (supervisor.rs:1893-1915) pins the F18 surface the ledger flagged.

**Weak spots (tests that would *still* pass after a bad change):**
- Re-running the *pre-F20* command-change window reset with a **constant** command passes
  `f20_stall_fires_exactly_at_stall_polls` and the pin-suites' stall test both (constant hold ⇒
  no reset trigger) — the killer is only the moving-command test. OK but single-point coverage.
- An "every second poll instead of every poll" AUTO retry passes `f20 test 3` unchanged (the two
  intermediate polls assert `pending: true`, true under both) — mandate 3's "genuinely every
  poll" is source-verified, not test-pinned.
- Changing `off_target_warned` to a repeating warn passes the whole suite (see finding 7).
- `f16_*` supervisor tests pass with F19's window deleted (instant echo mocks) — the F19 defense
  for the stall/L1 semantics rests on `settle_window.rs` + the two mock window tests alone.

## Fix class / assumptions that only hold on fixtures (mandate 4) — speculative section (labels preserved; all unproven)

1. **Settle-window read failure ≠ stale echo asymmetry (unproven hazard).** A read error on any
   sample aborts the whole window immediately (smc.rs:157 "a failed read aborts immediately") and
   surfaces as `SmcError::Read` → counted → 3-strike degrade; a *stale* echo, by contrast, is
   tolerated the full 1.5 s (and a re-issue). So an SMC that sporadically returns a bad read
   during verification degrades in 3 polls, while a slow SMC is tolerated for 3 s. Deliberate per
   the R4 comment, but nothing measures whether real applesmc sysfs reads fail transiently under
   load; if they do, curve mode pages/latches where the ruling presumably wanted tolerance.
2. **Echo adoption ≤ 1 s rests on one write of one experiment (unproven).** The 20:47 measurement
   was a single target (2000) on an idle machine. A target write that the SMC queues while busy
   (> 1.5 s) costs one re-issue (3 s total); adoption beyond ~3 s ⇒ `VerifyFailed` ⇒ 3-strike
   degrade. The F16 class re-enters by degree rather than kind; gate P5 (f) re-run is the missing
   evidence.
3. **Mock lag-mode register adopts instantly (smc.rs:689 `self.out_reg.set(target)` on the first
   sample in Manual)** — the F16-era supervisor tests that `set_tach_lag` *without*
   `set_echo_latency` still idealize register adoption; only `settle_window.rs` models the real
   async tick. Product code does not depend on instant adoption; the *tests'* realism does. Same
   family: the mock's `Auto` mirror rule (fan1_output mirrors SMC's own target, smc.rs:466-472,
   556-561) remains T9b finding 1 — still no real-hardware evidence, still not load-bearing.
4. **`read_fan` two-file snapshot torn read** (smc.rs:398-404, carried from T9b finding 4;
   self-corrects next poll — benign).
5. **stale-binary check has a blind spot the ruling does not name (observed live, not a code
   defect):** this box runs an F18-era binary whose unit start (20:37) postdates the installed
   binary's mtime (~20:20) → `doctor` reports `PASS — daemon started at/after the installed
   binary's mtime` while the *installed package* predates F19/F20 (the running daemon's
   state.json literally lacks `auto_restore_pending`). The check is exactly as ruled — but it
   can only ever detect "unit older than installed binary", never "package older than repo
   HEAD". One ledger sentence (reinstall + restart before gate (f)) is enough; flagging here
   because the live system is in precisely that state right now.

## Contract hygiene (mandate 6) — clean except findings 4-6

- **Appendix B:** `auto_restore_pending` present in both examples (state.v1 line 213,
  status.v1 line 196) — commit 80fe296 ✓; schema ids unchanged (`afanctl.state.v1` /
  `afanctl.status.v1`); state publish includes the field (supervisor.rs:852) and status JSON +
  human marker (cli.rs:732, 777-778) render it accurately ✓.
- **Frozen signatures:** no public signature changes in F19/F20; `MockSmc::set_echo_latency` is
  the ruled addition; `l2_absent` is supervisor-private (the N-F20-1 QUESTIONS.md entry correctly
  records why `panic_fd()` is not re-polled per-apply: it would break the frozen mock semantics).
- **unsafe:** confined to safety.rs (grep across src/); product-code `unwrap/expect/panic` = 0 as
  enforced by `[lints.clippy]` warn + `-D warnings` (T9-F4 confirmed live).
- **Dependencies:** exactly the R11 allowlist (serde, serde_json, toml, thiserror, tracing,
  tracing-subscriber, libc); no additions in the diff ✓.
- **LOC budgets:** F19 net +223 vs +180 (24% — stop-and-reported as N-F19-1 with drivers, accepted
  standing precedent); F20 net +158 vs +250 ✓ inside budget. Per-file §7 overage remains the
  standing accepted deviation.
- **Docs vs behavior:** the README's new L1 / failed-AUTO-retry / reconcile paragraphs match
  observed behavior except finding 6's one clause; `doctor`'s stale-binary check verified live
  today on the real unit (PASS line rendered above; non-root FAILs are the documented F11
  shape).

## Provenance notes

- All probes (`/tmp/opencode/t9c-probes/{a,a2,b,b2,c}`) ran the debug binary against tempdir
  copies of `tests/fixtures/sysfs/`, with `AFANCTL_RUNTIME_DIR` tempdirs; all probe files and
  processes were removed. The repo fixture was never touched.
- Read-only hardware observations: 20 × 1 Hz `fan1_input` samples (deltas 0–26 rpm), `doctor`
  (no `--roundtrip`), `systemctl show` fields, journal/state of the installed daemon. The
  installed daemon was not restarted, stopped, or written to; `fan1_manual` was never written
  outside the L2 handler's own reaction to my SIGTERM on a *probe* daemon, and one SIGKILL run
  verified the strand persists (probe B, second run).
- Every unproven claim above is labeled **unproven** or **speculative**; arithmetic claims
  (finding 2, worst-case poll) are derived from named code lines and measurements available in
  this turn and labeled where systemd-level enforcement was not exercised.
