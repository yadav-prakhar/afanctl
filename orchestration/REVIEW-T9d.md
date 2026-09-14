# T9d — adversarial review gate, round 4 (FINAL): F21 + F22, and the review-gate close/no-close judgement

Reviewer: T9d agent, read-only, branch `review/t9d`, head at `d32e254`. Subject: everything merged
after the T9c review — commit `1c8d033` (**F21**, RULING F21 R1–R4) and `aa93070` (**F22**, RULING
F22). Method: read `DESIGN.md` (BINDING), `orchestration/LEDGER.md`, `orchestration/REVIEW-T9b.md`,
`orchestration/REVIEW-T9c.md`, the two ticket cards and `PRD.md`; read the full `e1caf2f..HEAD` diff
(`src/supervisor.rs` +268/−37, `src/cli.rs` +52/−20, `src/config.rs` +33/−18, `src/policy.rs` +4/−2,
`tests/pin_constants.rs` +202, `tests/schema.rs` +9, README + DESIGN + packaging); ran all four §8 gates; then attacked each ruling
through source-path falsification, the shipped pinning tests' kill-shape, and independent arithmetic
(this machine has a real applesmc — see Provenance for why no live probes were run this round). No
probe of this review touched real `/sys`; the repo fixture was never actuated.

## Gate evidence (this checkout, this turn)

- `cargo fmt --check` — pass.
- `cargo clippy --all-targets --all-features -- -D warnings` — pass (14.99 s, zero warnings;
  the crate-level `[lints.clippy]` unwrap/expect/panic denial live, T9-F4).
- `cargo test` — pass: **162 tests** (132 unit + 11 integration + 7 traces + 6 pin_constants +
  4 schema + 2 settle_window), 0 failures.
- `cargo test --features hw` — pass; `hw_guard_skips_without_hardware` skips cleanly
  (`AFANCTL_HWTEST` unset on this box even though a real applesmc is present — the guard was
  exercised for real and no hardware writes happened from the gate).
- Suite wall-clock spot-check: `tests/settle_window.rs` still finishes in 8.11 s (test 2 = 3
  failing-echo polls — unchanged since T9c).

---

## Verdict: PASS — review gate: **CLOSE**

Both tickets match their frozen rulings, the ruling tests genuinely pin them (kill-analysis below),
and the round-3 findings all closed. This reviewer then hunted for residual holes along *every* path
that clears `manual_armed`, recomputed the watchdog arithmetic from the current source (the numbers
in the ruling's own doc comment are slightly stale — see finding 4), attacked the `polls`/`uptime_s`
arithmetic, and re-ran the T9b-style regression-killing attack on F16/F19/F20 semantics. Nothing that
makes the fan unsupervised or untruthful remains that this gate can act on: the survivors are
INFORMATIONAL only — one stale doc-comment figure (finding 1), one plugin-doc-honesty note
(finding 2), one cosmetic boundary coincidence (finding 3), and the kernel-timing note
(item 8, fixture-shaped section) the hardware gate rather than the review gate must carry.
Gate (f) may be declared clean.

Findings: **0 MAJOR, 0 MINOR** — 3 INFORMATIONAL (findings 1–3) plus the card-mandated
recomputation (finding 4) whose only defect-class residue is finding 1's doc drift. Every claim
carries a file:line; unproven claims are labelled **unproven**.

---

## Findings (prioritized)

### 1. INFORMATIONAL (doc-vs-code drift, no behavioural effect) · `src/config.rs:39-48` — the `MAX_INTERVAL_S` doc comment cites the *single*-window worst case, not the two-window poll worst case

**What.** The doc comment says "F19's settle windows cost ≈ 2.7 s per failing `write_speed` … and
≈ 1.8 s per failing `set_mode`" and concludes "a 12 s interval plus its own blocking keeps ≥ 3 s of
margin". The ruling's brief used the same figure (F21.md:38, `≈2.7 s`), but the worst case inside
*one poll* is higher: at `interval_s = 12` a poll whose `act` step hits a failing
`write_speed` (≈3.0 s ceiling) **plus** an L1 mode-drift re-assert `set_mode` failing (≈2.0 s ceiling)
blocks ≈5 s, and a poll in which *all four* write paths fail (apply-mode set_mode, act's
write_speed, L1's mode re-assert, L1's tracking re-assert) reaches ≈10 s (finding 4) — far more
than the 2.7 s cited (`src/supervisor.rs:219, 251` for the pings bounding it). The 12 cap itself
remains safe, but the *reasoning printed in the
code* under-counts the blocking it was calibrated against. The commit message quotes the same 2.7 s
figure. With 12 (not 14) the margin absorbs the under-count: the gap a watchdog sees is bounded by
`max(interval, in-poll work)` per the two-ping scheme — see finding 4's recomputation.

Reproduction: arithmetic only, from `smc.rs:147-211` (`window_ms = 1500`, 9 × 150 ms sleeps,
`WRITE_RETRY_MAX = 1` → 2 windows) and `smc.rs:725` (mode window 1000 ms). No live reproducer
exists on the fixture-only path for a *systemd-enforced* gap; the stand-ins are the pinned values
plus the unchanged `tests/settle_window.rs` wall clock quoted in Gate evidence.

Why it matters: the constant is *pinned by value* (`tests/pin_constants.rs` `f21_constants_are_pinned_by_value`
asserts `MAX_INTERVAL_S = 12`) and 13/14 rejection is behaviourally pinned
(`f21_max_interval_cap_12_legal_and_14_rejected`, `rejects_interval_starving_the_watchdog_budget_f3`),
so an editor slip cannot hide — but the comment is the only place the ruling's reasoning lives, and
it is internally inconsistent with itself (2.7 s cited, ≥5 s (up to ≈10 s) actually boundable). One sentence.

Direction: amend `src/config.rs:39-49` to the true worst case (failing-echo `write_speed` ≈3.0 s +
a failing mode re-assert ≈2.0 s ≈ 5 s in the common failure ceiling, ≈10 s all-paths-failing; the
two-ping scheme then bounds any
watchdog-observed gap by `max(interval, poll_work)` ≤ 12 s). Can ride a doc update; not
worth a ticket alone.

### 2. INFORMATIONAL (unproven hazard, fixture-shaped) · `src/supervisor.rs:504-535` — F21 R1's pending-observe mode field says `observe` while the release never verified (render honesty is carried by `auto_restore_pending` alone)

**What.** Under the failing-release branch the published `mode` is `"observe"` (the requested intent,
per ruling: "the mode field still becomes Observe") while the fan is still `fan1_manual = 1`. The
state file renders `mode: observe, verified: true, monitor_only: false, auto_restore_pending: true` —
truthful *only if the consumer reads `auto_restore_pending`*. Human `status` prints the
`auto_restore_pending` marker (cli.rs:781-782) and the README documents the flag, so this is
the contract as ruled, not a defect; but a plugin (omafan) that renders `mode` + `monitor_only`
*(the two fields dashboards have always keyed on)* shows a clean observe while the fan is
stranded Manual-at-3000-rpm until the retry completes. No code path reaches a latch in this state:
`fail_write` counts only reach
`WRITE_FAIL_FALLBACK = 3` if the retry's own failures accumulate — but the retry performs one
best-effort attempt per poll and does not itself feed the ladder: the observe branch calls
`fail_write` exactly once at the trigger (`src/supervisor.rs:528`), and the every-poll retry
`src/supervisor.rs:833-868` does NOT call `fail_write` on continued failure — it only
rate-limited-logs. So
`write_failures` sits at 1 and `monitor_only` stays false for the entire pending excursion,
whatever its length. Unproven hazard: **a plugin that ignores the additive field renders the wrong
story indefinitely** on a permanently-dueling-controller machine. The README's "A plugin must render
the latch, not the mode alone" (README.md:213-215) already covers the monitor-only latch but not
this one; the observe-pending case is the *same class* and the paragraph (README.md:206-210: "Failed
AUTO restore is never silent") does name `auto_restore_pending`. Judgement: contract satisfied, name
it for the plugin-facing doc; no code change recommended at this gate.

Reproduction: n/a — code-path read (`src/supervisor.rs:525-528` arms, `833-868` retries but never
counts; `verified` at supervisor.rs:244 stays `!monitor_only && l1_ok` and `l1_poll` early-returns
`true` under `auto_restore_pending` at 666-671 — so `verified: true` during a *failed, still-owned*
release. That `verified: true` is the sharpest edge: a scan of state could read it as "healthy").
Unproven: whether any real plugin does this.

Direction: one README clause ("while `auto_restore_pending` is true, `mode` may read observe with
the fan still Manual; the pending flag is the truth") — or, if the orchestrator wants the render to
defend itself: have `publish_state` report `verified: false` whenever `auto_restore_pending` is
true. The latter is a semantics change and would need a ruling; this reviewer does **not** recommend
it now (the flag + marker + the F21 test pin are sufficient and the change would break
`f21_failed_observe_release_keeps_ownership_and_retries`'s state asserts legitimately only as a
deliberate re-spec).

### 3. INFORMATIONAL (cosmetic) · `src/supervisor.rs:716` — the repeating dwell WARN's `is_multiple_of(30)` cadence makes one boundary convergence cost a count

**What.** The counter is reset on degradation (supervisor.rs:742, 793) and on convergence (752), and
the warn fires on every multiple — so one exact per-30-poll inside-tolerance dip coincides with the
boundary and restarts the window, effectively costing one count from the next warn's arrival
(closing-then-reopening inside one poll pair loses one count). The once-per-lifetime shape T9c's
finding 3 attacked is dead (proven below — R3 section); what remains is that a boundary-poll
convergence pins the next warn to exactly 30 fresh polls rather than 30-after-that-poll. Genuine
cosmetic; the mandated "repeats every OFF_TARGET_WARN_POLLS while the excursion persists" and the
"resets only on convergence or degradation" clauses are both satisfied by the code as written.
No direction.

### 4. INFO — the recomputation the card mandate asked for, in full (its only defect-class residue is finding 1's doc drift)

**Claim under test (T9c finding 2 / RULING F21 R2):** "the two-ping scheme actually bounds the gap
the reviewer computed in T9c (worst-case poll work ≈2.7 s at `interval_s = 12` vs `WatchdogSec = 15`)".

**Recomputed from current source.**

- The watchdog loop: `run()` → `loop { step_once(); sleep(self.interval) }`
  (`src/supervisor.rs:291-294`). Two pings: at `step_once`'s first line (219) and its last (251),
  via `ping_watchdog` (872-877). `step_once` never sleeps (the doc comment at 206-208 and the code:
  every `sleep` in `smc.rs`'s `settle` is bounded inside the window, ≤ 9×150 ms ≈ 1.35 s per window,
  `smc.rs:151-155`).
- Per-failing-call window cost: `settle_write` = (write + window) × (1 + WRITE_RETRY_MAX)
  = 2 × (write + window ≤ 1500 ms) ≈ **3.0 s ceiling** per failing `write_speed` (the F21 doc's
  ≈2.7 s figure uses 9×150 ms + the write; either reading is ≤ 3.0). `set_mode` uses
  `MODE_SETTLE_MS = 1000` → ≤ 2.0 s.
- **Worst poll at `interval_s = 12`:** one `Apply mode` `set_mode` (failing) ≈ 2.0 s + `act` →
  `write_controlled` failing `write_speed` ≈ 3.0 s + L1's mode re-assert (failing) ≈ 2.0 s +
  L1's tracking `write_speed` re-assert (failing) ≈ 3.0 s ≈ **10.0 s of blocking inside one poll**.
  With the two-ping scheme the *maximum interval between two consecutive pings* is
  `max(interval, in-poll blocking ≤ 10 s)` = 12 s — the start ping lands *immediately before* the
  blocking work and the end ping immediately after, so no watchdog-observed gap can contain both
  the 12 s sleep and the 10 s work. Gap ceiling = 12 s < 15 s. Margin ≈ 3 s — the ruling's
  requirement ("a poll period without ≥3 s of headroom crash-loops") is exactly met.
- **Live-validation note:** the original T9c finding validated the ≈2.7 s figure with a measured
  `tests/settle_window.rs` wall clock; the same wall-clock evidence (8.11–8.13 s for three
  failing-echo polls, unchanged from T9c and re-measured this turn) is consistent with
  `src/smc.rs:147-211`'s window arithmetic — 3 polls × 2 windows × ≤1.5 s. No systemd-level
  execution with `interval_s = 12` was performed (**unproven** in the live-systemd direction:
  this box's installed unit is deliberately not interfered with, and a 12 s config +
  dueling-writer fault cannot be honestly reproduced on the fixture-only path). The margin math
  itself is pinned by `f21_max_interval_cap_12_legal_and_14_rejected` and
  `f21_long_poll_still_pings_at_its_start` (supervisor.rs:2002-2014) — the behavioral
  "start ping precedes the poll's own blocking" pin.
- The symmetric corner: an operator choosing a *small* `interval_s` (e.g. 1) with long in-poll
  blocking is bounded the same way (`max(1, 10) = 10 < 15`) ✓.

## F21 R1 — FALSIFICATION ATTEMPT: every path that clears `manual_armed`

The card's strongest question — "can the fan still end up in Manual, unsupervised, with a
healthy-looking state, through *any* channel?" — was attacked branch by branch. `manual_armed` is
assigned at exactly 7 sites (`src/supervisor.rs:332, 355, 508, 571, 615, 796, 857`), plus the
observe-Err branch (515-528) whose correctness is precisely the *absence* of any assignment
there. Each:

| Site | Path | Safe? |
|---|---|---|
| 332 | `reconcile_stale_state` Ok — a **verified** `set_mode(Auto)` just succeeded | SAFE — verified only (R1 discipline) |
| 355 | `degrade_startup_to_observe` — from the reconcile's failed restore | SAFE for the *variable* — the fan was never owned by this process (`manual_armed == false` from `new()` at supervisor.rs:181, and nothing before this line arms it); `monitor_only=true` blocks act() entirely (supervisor.rs:562-563). The *physical* Manual-hung fan is not re-recalled here (the every-poll retry is **not** armed: `auto_restore_pending = false` at 360) — judged in residual (a) below. |
| 508 | `apply_mode` observe-branch Ok | SAFE — released via the verified path (the smc guarantee). |
| 515-528 | `apply_mode` observe-branch Err — **the F21 fix** | NOW SAFE: `manual_armed` stays true (no assignment in the Err branch at all), `auto_restore_pending = true` (525), attempts Armed, and the every-poll retry (833-868) owns the release. |
| 571 | `act()` ReturnToAuto Ok | SAFE — verified. Err at 575 does not touch `manual_armed` (correct since F20). |
| 615 | `enter_manual` Ok | arming, not clearing. |
| 796 | `degrade_to_auto` Ok | SAFE — verified. Err (809-822) keeps the fan armed and arms the retry (F20 R2's fix). |
| 857 | `auto_restore_retry` success | SAFE — the retry only clears when the register read `Auto` directly (supervisor.rs:838-841) or `set_mode(Auto)` returned `Ok` (verified, 842). |

Also traced: the `monitor_only` re-arm path at 487-499 clears `auto_restore_pending` on a fresh
command — the comment at 492-495 (RULING F21's "re-commanding is the fresh owner decision") is
honest: re-commanding *into* a writing mode clears the pending *release*, and the `manual_armed`
staleness is irrelevant because `enter_manual` is what (re)arms it next (591). And a foreign writer
channel (per the mandate): nothing in `l1_poll` can silently release the fan — under
`auto_restore_pending` the L1 section returns early (666-671) as ruled, and otherwise the mode-drift
check (673-678) re-arms against a foreign Auto.

**Two-path residual (both judged benign-as-ruled, named for honesty).**
- *(a) The reconcile failure path concedes the physical stranding.* `reconcile_stale_state`'s own
  restore failure (supervisor.rs:339-346) leaves a foreign-or-predecessor Manual fan parked, with
  `monitor_only: true` *but the every-poll retry NOT armed* (`degrade_startup_to_observe` sets
  `auto_restore_pending = false`). This is F14's ruling as it was designed ("failure ⇒ observe +
  monitor_only"), and T9c's coverage of `apply_mode`'s F20-R2 arm did not and does not cover this
  startup-only branch. Judged a **real residual**, not a falsification, because
  (i) the ruling froze this exact shape (DESIGN.md:160-166), (ii) on the real backend the same
  writability failure that made restore fail makes every later write fail identically, (iii) the
  state renders `monitor_only: true, auto_restore_pending: false` (truthful), and (iv) the *next
  restart's* reconcile re-attempts — L2/L3 still cover. **unproven** whether a "writable later"
  working set exists at all on the real backend (that is T9b's finding-3 shape, already classified
  minor). No change.
- *(b) The cmd-file channel's other exits.* `f21_auto_restore_retry_genuinely_attempts_every_poll`
  (supervisor.rs:1924-1953) kills an alternate-poll retry. There is no path through `apply_mode`'s observe
  transition other than the F21 arm; `f20_l2_absent_refuses_cmd_control` plus the `l2_absent`
  guard (480-486) shut the no-L2 re-arm path on the cmd channel.

## F21 R2 — FALSIFICATION ATTEMPT: does the two-ping scheme actually bound the gap?

**Claim to falsify:** "pinging at the start and end bounds the ping gap by `max(interval,
poll_work)`, keeping `interval_s = 12` under `WatchdogSec = 15`."

**Setup.** systemd's watchdog semantics (from `systemd.service(5)`): "the service must regularly
send a WATCHDOG=1 notification within WatchdogSec" — the *gap between successive notifications* is
the quantity the unit judges. Decomposition from the code:
`run()` (`supervisor.rs:266-295`): `loop { step_once(); sleep(interval) }`, ping at the head and
the tail of `step_once` (219, 251). Since 251 (end-ping of poll N) and 219 (start-ping of poll
N+1) are adjacent around nothing but the sleep, there are exactly two inter-ping distance classes:
  end(N) → start(N+1) = sleep ≤ interval ≤ 12,
  start(N+1) → end(N+1) = poll_work ≤ ceiling.
So the watchdog's *gap* view is: max(interval, poll_work ≤ ceiling). The card's own worst
case (finding 4 above) computes that ceiling ≈ 10.0 s < 12 s — so both inter-ping distances
(≤12 and ≤10) are < 15. **The scheme's bound holds.**

**Where the ruling's sentence is over-optimistic but correct-enough:** the doc comment at
supervisor.rs:210-214 and config.rs:45-48 quote "≈2.7 s per failing write" (the single-window cost)
while the full "both-flows-failing" poll is ≈4.5–10 s (finding 4's fuller sum). The bound that
matters for systemd is *each of the two inter-ping distances*, and the two-ping scheme is what
keeps the *work* distance short — that argument wins whether the work number is 2.7 or 10. The
semantic overstatement I flagged (finding 1) is cosmetic; the safe relation claimed ("12 keeps
≥3 s margin") survives under the more honest arithmetic.

**Installed-unit note (from T9c's live evidence, not re-probed this turn):** this box runs an
F18-era binary — the T9c review observed its running daemon's state.json lacking
`auto_restore_pending`, and no package install has occurred since. Such a state file also lacks
`polls`, i.e. this machine carries exactly the legacy population the fallback compatibility path
(finding 4's F22 section) was designed for. The unit was not disturbed (no restart, no writes) for
this review, per the mandate.

## F21 R3 — FALSIFICATION ATTEMPT: repeating dwell WARN, + cadence pins

- **Code path**: `off_target_polls` accumulates whenever armed and off target
  (supervisor.rs:702-703); the `off_target_warned` latch from F20 R4 is *gone from both the
  struct and the diff* (`grep off_target_warned` → absent); the warn fires on
  `is_multiple_of(OFF_TARGET_WARN_POLLS)` (716), which repeats, and *also* matches the T9c
  finding 3's "re-warn at 60, 90…" ask exactly.
- **Resets only on convergence (752) or degradation (742/793)** — as ruled (F21.md:57).
- **Tests kill-shape:** `f21_super_epsilon_jitter_never_stalls_but_warns_repeatedly`
  (tests/pin_constants.rs, hot side, asserts `dwell_warns ≥ 2` over 2×OFF_TARGET_WARN_POLLS with
  a parked, 60-rpm-jitter fan and `monitor_only` false throughout) — a re-latched (once)
  implementation fails (only 1 warn over 2 windows). The sub-epsilon pin
  (`f21_sub_epsilon_jitter…`) kills epsilon < 26.
- **Cadence value pinned**: `OFF_TARGET_WARN_POLLS = 30` in
  `f21_constants_are_pinned_by_value`; the warn count at poll 30 (and 60) is pinned in
  `f20_off_target_dwell_warns_at_window_and_resets_on_convergence` (2065-2122) and the two
  jitter pins. An editor's slip on 30 → 60 is caught by value, and 30 → 31 by the resolution of
  "warn fires exactly at OFF_TARGET_WARN_POLLS" (f20 test 6).

## F22 — FALSIFICATION ATTEMPT: `polls` / `uptime_s`

- **"once per completed poll, advanced before publishing"**: `step_once` advances
  `self.polls += 1` (248) *then* `publish_state` (249) — the published number *includes* the
  current poll; the `once` verb therefore publishes `polls: 1` for its single poll
  (tests/schema.rs:275-280 asserts exactly that). ✓ per ruling ("the number of completed polls
  since the daemon started, one per poll").
- **`uptime_s` exact**: `cli.rs:736` prefers `polls`, `saturating_mul(interval_s)`. Test 2 pins two
  intervals (42 × 1 = 42; 42 × 3 = 126; cli.rs:1315-1339) ✓; the wrong-arithmetic test was
  corrected rather than added-around (cli.rs:1265, 1280-1281) ✓.
- **Fallback reachability with a *current* state file**: the claim is "`polls` can't be missing
  while `watchdog_pings` is present, for any state file this build writes". `publish_state`
  (StateFile, supervisor.rs:128-146 and 898-910) serializes both fields unconditionally — `polls` cannot be absent when
  `watchdog_pings` is present, because `serde_json::to_string` serializes a struct with two
  non-Option fields (no `skip_serializing_if`) — **proven**. The `.or_else` (cli.rs:736) therefore can only fire on a
  legacy build's file. ✓ Exactly as documented (cli.rs:728-735, README.md:216-224).
  **Residual (cosmetic, `unproven` hazard — but a strictly narrower population than the design
  intent targets):** an F21-era intermediate build
  (`watchdog_pings` = 2N) falling back yields 2×-uptime — the population is
  "daemons built between the F21 merge and the F22 merge", packages built in that interval exist
  on exactly this machine class (this box demonstrably runs an F18-era binary), and the README
  names the backward-compat shape as "correct for that build, where there was one ping per poll"
  — which is **false for the F21-era build** (F21 *did* two pings per poll already, so its state
  files' `watchdog_pings ÷ interval_s` → ~2× the true value). That transient is a *read-only*
  client of a state file that the F22 daemon overwrites on its first poll — so the wrong figure
  lasts one poll interval after upgrading. Cosmetic; a README clause ("an F21-era build's
  `watchdog_pings` may be 2-poll; that build is superseded") closes it. **Not a defect worth
  holding the gate open.**
- **Schema ids unchanged**: `tests/schema.rs` asserts `afanctl.state.v1` with `polls` present,
  additive (182-204) ✓; Appendix B example updated (DESIGN.md:213-218) ✓; Appendix A's
  `step_once` doc comment now documents the two pings (DESIGN.md:154-158; N-F21-2's closure ✓).
- **`run()` reset**: `self.polls = 0` at run() entry (267-269), never loaded from disk ✓ —
  kill-analysis: removing this line cannot be caught by `run()` tests (none exist that call
  `run()`), and `Supervisor::new` initializes `polls: 0` (supervisor.rs:194-206) so a fresh daemon's state is
  right anyway; the line is defense against CLI reuse *within one process* (`once` builds a fresh
  Supervisor per invocation, cli.rs:453-462, so no shortcut exists either). Harmless.

## Regression risk (mandate 2): kill-analysis, F21/F22 vs F16/F19/F20 semantics

**F21 revert-killers (all rulings hold):**
- R1 → `f21_failed_observe_release_keeps_ownership_and_retries` (supervisor.rs:1861-1914) dies
  (asserts `manual_armed` stays true + `auto_restore_pending` arms + no "cmd applied" + the
  factual `recent_errors` entry + the fan still Manual; then the fault clears and the retry
  releases with "AUTO restored after 3 attempts" — covering exactly the healthy-looking-while-stranded
  render the T9c MAJOR turned on).
- R2 → `f21_watchdog_pings_at_start_and_end_of_each_poll` dies (asserts `2N−1` i.e. two pings per
  poll, start counted by publish, end not); `f21_long_poll_still_pings_at_its_start` +
  `f21_constants_are_pinned_by_value` (`MAX_INTERVAL_S = 12`) + `f21_max_interval_cap_12_legal_and_14_rejected`
  + `config.rs` `rejects_interval_starving_the_watchdog_budget_f3` (14/15/20 rejected, "at most
  12 s" in the fix message). Unpinned underneath: nothing.
- R3 → `f21_super_epsilon_jitter_never_stalls_but_warns_repeatedly` dies (≥2 warns over
  2×OFF_TARGET_WARN_POLLS, never degraded); sub-ε pin covers the ε boundary.

**F22 revert-killers:** `f22_polls_once_per_poll_and_pings_twice` dies if either counter stops
being distinct — pins `polls = N` **and** `watchdog_pings = 2N` simultaneously, so the pre-F22
`uptime_s = watchdog_pings × interval` defect is dead; the corrected unit + `status_uptime_s_*`
trio (cli.rs:1261-1361) kills every fallback perversion; `tests/schema.rs` `assert_state_v1` requires
`polls` present-and-u64 with the schema id unchanged.

**F16/F19/F20 semantics untouched (the `e1caf2f..HEAD` diff touches only what the mandate
names):** `smc.rs` is read-only to F21/F22 — zero `src/smc.rs` changes (the diff is empty), so the
settle window, the echo tolerance, the tracking/drift split, `MockSmc`'s fault face and
`set_echo_latency` are byte-identical to the T9c-reviewed tree. `policy.rs` changes only the
doc-comment of `OFF_TARGET_WARN_POLLS` and nothing else (diff +4/−2, all comment). Supervisor's
F19/F20 machinery (`l1_poll`'s three checks, `degrade_to_auto`, `auto_restore_retry`, the stall
window) reads unchanged apart from: the `off_target_warned` latch removed (R3), the observe branch
rewritten (R1), and the ping call duplicated (R2) — every relevant T9b-c-kill test still in the
tree and green (policy_traces, pin_constants, settle_window, the f16/f20/f19 families).

## Anything still fixture-shaped (mandate 3) — the handoff honesty chapter

Carried from T9b/T9c with labels preserved; two *new* possibly-fixture-shaped items added.

**Not load-bearing (proven or benign):**
1. **`read_fan` two-read torn snapshot** (`smc.rs:398-404`) — carried since T9b finding 4;
   self-corrects next poll; benign. Hardware gate **can** cover it (it is exercised by every
   read under the real 1 Hz tach).
2. **Mock lag-mode single-sample register adoption** (`smc.rs:687-691` out_reg adopts on the *first
   sample*, then `echo` reflects) — T9c's mock-realism carry; product code does not depend on
   instant adoption; hardware gate **can** cover (the 20:47 measurement that produced
   `ECHO_SETTLE_MS` was exactly this defense against idealized adoption) — done already.
3. **Auto-mirror rule in the mock** (`smc.rs:455-461, 642-650; set at 561`) — still unproven, still not
   load-bearing (Auto writes never happen; T9b finding 1 lineage). Hardware gate **can** cover
   (gate (b) and (f) exercise the Auto restore whose observed read-back is the mirror).
4. **`fan1_input` jitter well below epsilon** (T9c's live measurement: 20 × 1 Hz deltas 0–26) — the
   0.5-ε real margin makes the `STALL_TACH_EPSILON_RPM = 50` edge safe. Hardware gate **can**
   cover (cheap one-shot re-sample in gate (f)).
5. **Register echo modelled as exactly-adopts-or-fails** (`settle()` reads the actual register
   file; every transient SMC anomaly is lumped into VerifyFailed by design, `settle_write`'s
   "one write per window" invariant holds structurally). Hardware gate **can** cover — and did
   (20:47), the very experiment that produced F19's settle window.
6. **"Sensor read failure during a settle window"** (T9c's speculative finding 1: a mid-window
   read error is a counted failure) — unchanged, `settle()` still aborts and returns the error
   (smc.rs:156-158). Hardware gate **can** observe whether real applesmc sysfs reads fail
   transiently under load, but *cannot* deliberately produce the failure on hardware; treat as an
   accept-as-designed residual.
7. **The failing-settle cost fuse (T9c's finding-2 arithmetic)** — by definition of F21 R2, and
   recomputed by this review (finding 4), safe at any legal interval. Hardware gate **cannot** add
   signal (the arithmetic is the code + constants). This is the one "only holds on the paper"
   item — it holds on *arithmetic*, not fixtures; labelled so.

**New from this round:**
8. **The watchdog-gap bound holds for a healthy poll with ≤ ~10 s of blocking *only inside
   `step_once`*.** `run()`'s `std::thread::sleep` (supervisor.rs:293) can itself sleep *longer* than
   `self.interval` under machine-wide scheduling stall (honest wall-clock overhead) — systemd's own
   restart discipline covers it, L2 restores AUTO, and RestartSec=1 crash-loops back; but the
   "gap ≤ max(interval, poll_work)" proof's sleep term is bounded by the *kernel*, not by our
   code, and this review's arithmetic (like the ruling's) treats the sleep as exactly
   `interval_s`. **unproven**: whether real-world systemd counts this accounting difference;
   the L1/L2/L3 safety chain is indifferent.
9. **The `--at-temp` `once` band**: `MockSmc::new(config.min_rpm, config.max_rpm)`
   (cli.rs:455) uses the *curve band* (default 1200–6200) rather than the hardware band — for
   a `--at-temp` value near the top of the curve the once path clamps differently than a live
   poll does. Pre-existing, untouched by F21/F22, out of this gate's subject, and conceivably
   intentional; **unproven** which. **Direction**: one sentence in README or a one-line
   `ResolvedConfig` plumb; not a gate-blocker.

## Contract hygiene (mandate 4) — clean

- **Appendix A (binding signatures + constants)** · `src/config.rs:36-49`'s `WATCHDOG_UNIT_SEC`
  and `MAX_INTERVAL_S = 12` are **not** in DESIGN.md's constants block (DESIGN.md:103-116 lists
  the policy.rs constants only, and does not mention the two config.rs watchdog constants;
  pre-existing shape since T9-F3 — the T9c ruling's statement "all ruled constants present (the
  list was just completed)" referred to the *policy* constants, and both config constants were
  already binding pre-F21). Not a regression, not a F21 delta; the two new F21-relevant items in
  DESIGN.md (`step_once` doc, Appendix B `polls` + `1962`) are both present ✓. RULING F21's
  "the constants list is updated there" (F21.md:4) — the F21-named constants have their values
  pinned in the code comments and value-pinned in tests (f21_constants_are_pinned_by_value).
- **Appendix B**: `polls` present in the state.v1 example (DESIGN.md:217) ✓; `watchdog_pings`
  example updated to `1962` = 2 × 981 ✓ (the two-ping per-poll arithmetic) ✓; schema ids unchanged
  (`afanctl.state.v1` L213, `afanctl.status.v1` L196); additive fields only, exactly the
  `monitor_only` / `auto_restore_pending` precedent.
- **`unsafe`**: confined to `safety.rs` (grep across `src/`; every block `// SAFETY:`-carried,
  `src/safety.rs:38, 57, 72`). ✓
- **Dependencies**: no new entries; `Cargo.toml` unchanged in `e1caf2f..HEAD`; the crate deps are
  exactly the R11 allowlist (serde, serde_json, toml, thiserror, tracing,
  tracing-subscriber, libc). ✓
- **`unwrap/expect/panic!` in product code = 0**: enumerated by the `#[cfg(test)]` module
  boundaries in every file (supervisor ≤996: only `unwrap_or_default`-family total-functions; cli
  ≤849: ditto; doctor ≤739: none; config ≤366: none; policy ≤292: none; notify ≤55: none; smc
  ≤753: `unwrap_or_default` only) — and clippy `-D
  warnings` passes with the crate-level denies live, which is the machine-checked form of this
  claim. `unwrap_or*` combinators are not the `unwrap_used` lint's subject; no `expect`/`panic!`
  outside tests. ✓
- **LOC budgets**: F21's card cap was ≤ +200 product LOC. Measured independently by hunk recount
  (excluding only hunks whose header names `mod tests`): F21 (`1c8d033`) net
  insertions−deletions = **supervisor.rs +39, config.rs +13, policy.rs +2 ≈ +54 upper bound**
  (an upper bound, not exact — the supervisor's added tests sit inside test hunks whose headers
  do not re-name `mod tests`, so some of the +39 is test code; the commit's own self-report says
  +19; every reading is well inside the budget). F22 (`aa93070`): cli.rs ≤ +4 net product
  (+52/−20 overall including the corrected tests) and supervisor.rs ≤ +16 net (StateFile field,
  counter, doc comments, and the F22 tests share hunks) — every reading is inside the card's
  ≤ +80. Per-file §7 overage
  (supervisor.rs 2147 / cli.rs 1427 / doctor.rs 1197 / smc.rs 1136) is the standing
  accepted deviation (Q-T5-3 etc.), unchanged by this diff's shape.
- **Docs vs behaviour**: README 1..=12 + "two watchdog pings per poll" (README.md:142-144,
  155-161) matches `config.rs:220` and `supervisor.rs:219, 251`; L1 paragraph now says "never
  moves **between polls**" + the repeating-WARN clause (README.md:21-27) matches
  `l1_poll`'s actual criterion (supervisor.rs:728-739) and the R3 warn (716-724, 752-754); the F22
  paragraph (README.md:216-223) matches `cli.rs:736` and the README fallback narrative exactly,
  including honesty about the legacy estimate; `packaging/afanctl.service` unchanged
  (`WatchdogSec=15`), consistent with `WATCHDOG_UNIT_SEC = 15`.
- **DEVIATIONS / QUESTIONS**: `N-F21-1` and `N-F21-2` in QUESTIONS.md were the
  cross-agent flags and are respectively *resolved by F22* (uptime path) and *closed by the
  orchestrator's DESIGN.md re-render* (Appendix A comment + B example both updated). DEVIATIONS.md
  has no F21/F22 entry: the F21 and F22 product deltas are inside their card budgets — nothing
  to stop-and-report. ✓

## Provenance notes (honesty)

- All four §8 gates were executed in this checkout this turn (numbers quoted above; 162 tests).
- **No live sysfs probes were performed by this review** — every question the mandate raised was
  answerable from source + the existing pinned tests + this turn's test runs, because F21/F22
  shipped direct unit-level observable assertions for each ruling (the ping-count pins expressed
  *as the published counter*, both jitter trade-off arms, the every-poll retry's write-attempt
  delta, the value pins). Where a T9c claim survived unchanged (settle window, stall criterion,
  `MAX_INTERVAL_S` semantics), source citation and recomputation replaced re-probing — the T9c
  report's live evidence for those surfaces stands and was not invalidated by any code this diff
  touched (`src/smc.rs` is byte-identical in `e1caf2f..HEAD`).
- The **installed unit** note: the F18-era vintage follows from the T9c report's live observation
  (its running daemon's state.json already lacked `auto_restore_pending` at T9c review time) — no
  package install has occurred since; this review did not stop, start, restart, write, or run any
  probe against it, and ran no hardware tests (`--features hw` verified only via the skip guard,
  `AFANCTL_HWTEST` unset).
- The repo fixture (`tests/fixtures/sysfs/`) and real `/sys` were never written by anything this
  review executed (the suites run against tempdirs/mock per the shipped tests); `git status` was
  clean at the start of the turn aside from this report file.
- Unproven claims are labelled inline: finding 2, finding 4's systemd-level note, and
  fixture-shaped items 6, 8, 9.

---

## Gate judgement (mandate 5)

**review gate: CLOSE.**

Reasoning: the last three rounds each found a MAJOR *proven live*; this round's mandate was to
falsify the fixes that closed them, and every falsification attempt terminated in a working
defense: the observe-channel hole (T9c's MAJOR) is closed with a both-channels test and a
falsifiable alternate-poll killer; the watchdog-margin corner (T9c's arithmetic) is closed
structurally and re-pinned by value; the repeating-warn visibility fix is pinned both ways; the
`uptime_s` arithmetic is now exact with a *reachable-in-practice-only-on-legacy* fallback; and the
F16/F19/F20 rules survived with zero semantic surgery (the `smc.rs` file is untouched by this
diff). The surviving informational items are (1) one stale doc-comment figure, (2) a plugin
doc-honesty clause (no code), (3) a cosmetic boundary coincidence, and fixture-shaped item 8's
unprovable-in-test kernel-timing note. None hides a fan-safety hole; none blocks gate (f). Per the
card's own standard:
a PASS that hides nothing beats a FAIL that produces nothing — **CLOSE** is the honest verdict.

Minimum fix list if the *orchestrator* wants more (not required to open the gate again):
- one sentence in `src/config.rs:39-49` measuring the true in-poll worst case (finding 1),
- one README clause for the F21-era build's `watchdog_pings ÷ interval` double-count reading for
  one poll interval after upgrading (F22 section, finding 4),
- optionally the `verified: false while auto_restore_pending` semantics (finding 2) — a
  deliberate re-spec, not a fix.

— T9d reviewer, 2026-09-14.
