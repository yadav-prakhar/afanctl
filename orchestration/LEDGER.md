# afanctl orchestration ledger — orchestrator-owned; subagents read-only

## Index
| id | title | model | branch | status | dispatches | bounces | timeouts | merged (sha) |
|----|-------|-------|--------|--------|------------|---------|----------|--------------|
| S0 | bootstrap | glm-5.3 (orchestrator) | — | MERGED | 0 | 0 | 0 | (initial commit) |
| T0 | scaffold+contracts | glm-5.3-flash/high | t0-scaffold | DISPATCHED | 1 | 0 | 0 | — |
| T1 | config | deepseek-v4.1-flash | t1-config | MERGED | 1 | 0 | 0 | b71a55e |
| T2 | policy | glm-5.3-flash/high | t2-policy | MERGED | 1 | 0 | 0 | aeb2b71 |
| T3 | smc | glm-5.3-flash/high | t3-smc | MERGED | 1 | 0 | 0 | 3b41477 |
| T4 | safety+notify | glm-5.3-flash/high | t4-safety | MERGED | 1 | 0 | 0 | 5773595 |
| T5 | supervisor | glm-5.3-flash/high | t5-supervisor | MERGED | 1 | 0 | 0 | a6bbdbd |
| T6 | cli+main (a: parser; b: wiring) | deepseek-v4.1-flash | t6-cli | MERGED(a) | 1 | 0 | 0 | 815d0fa |
| T7 | doctor | deepseek-v4.1-flash | t7-doctor | MERGED | 2 | 0 | 1 | a71b9f4 |
| T8 | integration+packaging | deepseek-v4.1-flash | t8-integration | MERGED | 2 | 0 | 1 | 9b6aa9d |
| T9 | review gate | glm-5.3-flash/high | t9-review | CLOSED | 1 | 0 | 0 | (tickets merged) |

<!-- statuses: PENDING · DISPATCHED · VERIFYING · BOUNCED(n) · MERGED · BLOCKED · ESCALATED -->

## Events (append-only, newest last)
- [2026-09-14] S0: smokes passed (deepseek-v4.1-flash OPENCODE_SMOKE_OK; --auto write path ok). rust 1.98.1 installed via pkexec. repo initialized (identity pkrc267@gmail.com). PRD.md + PLAN.md + orchestration/** committed.
- [wave 1] T1/T2/T3/T4/T6a all verified first-pass (gates green, ownership clean, porcelain empty).
  Merged T1→T2→T3→T4→T6a (rebases union-resolved on append-only D/Q files only).
  Adjudications: D-T6-1 ACCEPTED (doctor::run +json, DESIGN.md amended, T6 fixed on branch pre-merge);
  T3-N2 intrinsic layout hook blessed; T1 Q-T1-1 (strict missing-key refusal) + T6 Q-T6-1
  (AFANCTL_RUNTIME_DIR) now spec. LOC overages config.rs/smc.rs/cli.rs noted, within stop-and-report rule.

- [A1 pending] T5 next.
- [fix ticket F1] T4 notify tests race on NOTIFY_SOCKET process env (abstract_socket_form_reaches_listener
  flakes ~10-30%% under parallel test threads; passes serialized). File owned by T4. Bounce to T4 model
  with failing output. Discovered during T5 verification (T5's larger suite raised flake probability).
- [T5 verified] gates green; ~630 LOC supervisor.rs (planner-ruling parked; same class as earlier overages, $mandated feature set).
- [F1 fixed] t4-fix1 used OnceLock<Mutex<()>> env-lock serialization (shape (a), no sig change);
  5x consecutive green verified pre-rebase, 3x post-rebase; merged 4e5ee6e. 0 residual flake.
- [A1 NEXT] Wave-2 complete. A1 audits after T5 per PRD §11.4 — running next.
- [A1 PASS, 2026-09-14] audits: unsafe confined to safety.rs (4/4 SAFETY comments); /sys paths only in smc.rs;
  product-code unwrap/expect/panic = 0 (78 raw hits all inside #[cfg(test)] blocks — brace-matched verification);
  deps exactly the R11 allowlist; release binary smoke: --version ok, no-args exit 2 + usage,
  status reads fixture sensors + fan + defaults, hold writes cmd.json via AFANCTL_RUNTIME_DIR,
  once reaches verified SetSpeed against fixture. doctor stub exits 101 loudly (T7 owns body).
  LOC: supervisor.rs 1082 / smc.rs 887 / cli.rs 649 / config.rs 535 / policy.rs 336 — overages
  stop-and-reported by owners, deprioritized to T9 ruling.
- [incident, orch-caused] A1 smoke ran release binary 'once' against the REPO fixture dir;
  set_mode wrote fan1_manual=1 in the working tree → post-T6b-merge gates flipped RED with
  3 phantom failures. Fixed by `git checkout -- tests/fixtures/sysfs/`. Lesson recorded:
  orchestrator smokes must use tempdir copies (AFANCL_RUNTIME_DIR + copied fixture), never
  the repo fixture path, with any verb that actuates.
- [T6b MERGED] c0eaed4 (phase b wiring: daemon/once/hold real Supervisor, --at-temp, --dry-run,
  hold clamp-before-write, doctor stub catch → exit 1). Post-fix main gates GREEN.
- [T7 first-dispatch died silently mid-run (4 min, no commits); re-dispatch succeeded. T8 same.
- [D-T8-1 RATIFIED] selftest-panic arms L2 vs --sysfs-root; 9-line arm verified by failing→passing test.
- [T7 verified] 98 tests green; doctor card complete. [T8 verified] integration tests spawn real binary
  (tempdir fixtures, AFANCTL_RUNTIME_DIR), selftest-panic restores AUTO, packaging matches Appendix D.
- [A2 PASS] audits re-run: unsafe confined; /sys confined; deps allowlist exact; product-code
  unwrap=0 (rechecked); .omo noise untracked+ignored via T8 chore commit.
- [T9 VERDICT: FAIL — verified] F1 CRITICAL reproduced (once leaves fan1_manual=1, CODE=0);
  F2 reproduced (doctor ignores --sysfs-root/--config — read live /sys at 70C vs fixture 45C);
  F3 reproduced (interval_s=20 accepted, would starve WatchdogSec=15); F4 reproduced
  (unwrap/expect/panic lints not enforced; 128 violations appear when flags added — all in tests);
  F5 reproduced (negative thresholds → policy.rs:195 overflow panic in debug). Minors F6–F13 accepted
  as reported by T9 (repro consistent with code), to be batched. Fix tickets F1–F5 + polish ticket next.
- [FIX ROUND COMPLETE] Ticket A (F1 F2 F7 F8 F12) + Ticket B (F3 F4 F5 F6 F10 F11 F13-doc) merged;
  mechanical lint-attr completion for cfg(test)/integration files (cbb90b3). ALL findings re-verified live:
  F1 once→fan1_manual==0 CODE=0 ✓; F2 doctor reads fixture (t_eff 45.0, fixture provenance) ✓;
  F3 interval_s=20 rejected exit 1 key+fix ✓; F5 exit 1 key+fix no panic ✓; F4 lints enforced green ✓.
  Tests: 103 unit + integration + traces + schema, all green; all four §8 gates green on main.
- [STATUS] Build complete pending §9.3/§9.4 user-supervised hardware gate. Final report below.

## Final report (2026-09-14)
- 11 tasks (S0-T9) + 2 fix tickets (F1 notify-race, T9 fixes) executed by 11 subagent dispatches + 3 fix dispatches.
- Index shows: every task MERGED. Bounces: 0 quality bounces; 2 dispatch deaths (T7/T8 attempt 1, exit 140-adjacent, re-dispatched).
- Rulings: D1, D-T6-1, T3-N2, T1 Q-T1-1, T6 Q-T6-1, D-T6-1, D-T8-1, T9 F2-doctor-sig — all adjudicated & propagated.
- LOC: product ~3.5k (over budget, stop-and-reported, weighed by T9 as quality risk; not a defect).
- Remaining for user: §9.3 a-g checklist + makepkg -si (Appendix P5 in PLAN.md; ve PRD §9.3/9.4).
- [FINAL GATE SWEEP] hold-integration test used interval_s=30 (pre-F3) → dead after F3 cap; retuned to 10 s
  (still one long poll, now legal). ALL FOUR GATES GREEN on main at 8c6d2fd.
- [STATUS] Build complete. §9.3/§9.4 supervised hardware gate handed to user (Appendix P5, PLAN.md).
- [HW GATE (a) attempt 1 — 2026-09-14 13:48] user ran `afanctl doctor` as non-root: dispatcher WARN
  "cannot pre-open fan1_manual O_WRONLY; L2 death path unavailable (os error 13)" + 2 FAIL
  ("fan files present & writable", "L2 fd armed") + expected WARN (unit not installed). Adjudication:
  NOT a defect — matches README "Root required (F11)"; both FAILs are the same permission cause, exit 1
  by design. PLAN.md P5 item (a) amended to say run elevated (orchestrator-owned doc). Re-run elevated;
  no code ticket opened. All other checks PASS (3 sensors, t_eff 54.0 C, fan 1200..7200, config defaults valid).
- [HW GATE (a) attempt 2, root — PASS] 7 PASS + 1 expected WARN (unit not installed), exit 0.
- [HW GATE (b) — PASS] `doctor --roundtrip` root: "manual+1200 rpm verified over 2 s; AUTO restored
  (observed 1210 rpm / Manual)". Code-verified: the parenthetical is the during-hold snapshot read
  BEFORE the restore; PASS requires restore Ok AND observed mode == Manual (src/doctor.rs:433). No defect.
  Also confirmed by code: non-root `--roundtrip` refuses to write when `panic_fd()` is None.
- [HW GATE (c) — PASS] `selftest-panic`: panic at src/safety.rs:81, exit 101; `/sys/.../fan1_manual` == 0
  after → L2 death path proven on real hardware (the C1 defect class is dead).
- [NOTE] P5 ordering: `d` needs the installed unit, so §9.4 `makepkg -si` must precede `d`; P5 annotated.
- [HW GATE (d) — in progress] unit installed (§9.4 install half user-run); `systemctl start afanctl`
  → active (running), Main PID, `Status: "mode=observe"`, RSS peak 2.4M, NRestarts=0 after >15 s with
  WatchdogSec=15 (proves READY=1 via Type=notify AND live watchdog pings); `afanctl status` as the
  normal user reads /run/afanctl/state.json (root:root 0644) → plugin render feed works without sudo;
  fan 1658 rpm manual=false → observe writes nothing, SMC curve still owns the fan. Soak pending.
- [DEFECT F14 — CRITICAL, found by orchestrator during gate (d)] `Supervisor::run()` has **no startup
  reconcile**: SIGKILL (uncatchable) in curve mode kills the process with `fan1_manual == 1`; systemd
  restarts in observe mode, which writes nothing by design (R3) ⇒ nothing restores AUTO, fan stays
  manual at the last curve speed indefinitely and silently = **mbpfan C1 class resurrected**. PRD §9.3e
  ("L2 already restored AUTO") names a mechanism impossible for an uncatchable signal; its *property*
  is the real bar. Verified in code (src/supervisor.rs `run()` + `RunMode::Observe => Decision::Observe
  => None`) and in tests: integration covers SIGTERM only ("SIGTERM (not SIGKILL) so the L2 handler
  runs"). Fix ruled + frozen in DESIGN.md (RULING F14): arm L2 → reconcile (Manual ⇒ restore AUTO,
  verified write, loud log; failure ⇒ observe + monitor_only) → sd_ready → loop; plus one startup
  info! line (journal currently carries only systemd's lines, so §9.3d has no app-side evidence).
  Ticket F14 dispatched (glm-5.3-flash high). P5 (e) and (f) marked BLOCKED until it lands.
- [F14 — MERGED] branch `fix/startup-reconcile` (9c5f400) merged to main; gates green on merged tree
  (126 tests, fmt/clippy clean, `--features hw` guard skips). Verified by orchestrator, not self-report:
  `run()` order is arm L2 → `reconcile_stale_state()` → evidence line → sd_ready → loop; reconcile =
  Manual ⇒ verified `set_mode(Auto)` + loud warn/info + note_recent, failure ⇒ observe + monitor_only,
  Auto ⇒ zero writes. Tests present: 4 unit (single verified write / no write on healthy path /
  read-failure degrade / restore-failure degrade) + 2 integration (observe reconciles stale manual;
  curve reconciles then takes control). Package rebuilt (20:01) for reinstall.
- [RULING N-F14-1 — ACCEPTED (partial)] agent stopped instead of altering frozen `RuntimePaths` to carry
  the config-source path for the startup line; shipped line names mode, L2 armed, notify path, state file,
  hw band. Accepted: config provenance stays visible via `status --json`, and the field's value in the
  journal is marginal. The proposed `RuntimePaths{+config_source}` change is parked for the F15 bundle
  (same file, cli.rs) if the user wants F15 — no separate signature change now.
- [MINOR F15 (deferred, user's call)] non-root `afanctl status` prints the smc discovery WARN
  "cannot pre-open fan1_manual O_WRONLY; L2 death path unavailable (os error 13)" before its human
  output. Honest but misleading on a read-only path the plugin/polkit rule is designed to allow;
  candidate fix: emit it only when the caller can write, or demote to debug for read-only verbs.
- [HW GATE §9.4 build half — PASS, orchestrator-run] `makepkg --nodeps` (no install; safe, no root, no /sys)
  built afanctl-0.1.0-1-x86_64.pkg.tar.zst cleanly. Contents verified: usr/bin/afanctl,
  usr/lib/systemd/system/afanctl.service, etc/afanctl/afanctl.toml with `backup = etc/afanctl/afanctl.toml`,
  usr/share/polkit-1/rules.d/49-afanctl.rules. Cargo.lock tracked (`--locked` build valid). gdb-add-index
  notice is cosmetic (release build has no debuginfo). Install half remains user-gated.
- [DEFECT F16 — CRITICAL functional, found on real hardware while starting gate (f)] `sudo afanctl curve`
  at 20:06:19 → daemon latched monitor-only ~2 s later. Journal: 3× "fan1_output write not taken by
  hardware; retrying … wrote=1200 read_back=6170/5235/1866" then "L1 fallback: 3 failed writes → AUTO
  restored, degrading to monitor-only". Root cause (source-verified, not inferred): (1) `src/smc.rs:339`
  verifies a `fan1_output` write against **`fan1_input`** (the tachometer) with ±150 rpm — a fan still
  spinning down from 6.2k can never match a 1200 command, so every ramp and every takeover reads as
  "write not taken"; (2) `src/supervisor.rs:514-600` `l1_poll` counts that same physical deviation as a
  write failure → 3 polls → AUTO + monitor-only latch. Net: on real hardware curve mode disables itself
  the instant the target moves >150 rpm. Fail-safe direction held (AUTO restored, loud logs) but the
  primary feature was non-functional. Live context: SMC's own curve had the fan at ~6.2k rpm (its raw
  TC0F 63.5 C) while our curve wanted 1200 rpm (coretemp t_eff 48-51 C). Missed by 126 tests and T9
  because `MockSmc`/fixtures echo a write into `fan1_input` instantly — idealized physics.
  RULING F16 frozen in DESIGN.md; ticket `orchestration/instructions/F16.md` dispatched (glm-5.3-flash
  high): echo-verify against `fan1_output` (±50 rpm), L1 split (mode drift = counted failure; tracking =
  re-assert, not counted), `STALL_POLLS = 10` stall detector for a genuinely dead actuator, and fixtures
  gain tach-lag dynamics + the SMC-mirrors-its-own-target rule. P5 (f) marked BLOCKED until it lands.
- [STATUS] Gate (e) not yet run (needs F14 reinstall, done) ; gate (f) blocked on F16; (d) soak unaffected.
- [RULINGS — user approved 2026-09-14] (1) PRD amended in four places to match the frozen rulings:
  §9.3e now names the startup reconcile (SIGKILL is uncatchable), R1 clarifies that read-back is of the
  attribute just written (`fan1_input` is never a write-verification source), R4-L1 documents the split
  checks + stall detector, Goal 2 notes uncatchable deaths are covered by the reconcile. (2) F18 bundle
  approved (F15 + F17a + F17b + N-F14-1) → `orchestration/instructions/F18.md` rendered, held until F16
  merges (same files: smc.rs/supervisor.rs/cli.rs). RULING F18 freezes: additive `monitor_only` field in
  state.v1 + status.v1 (no id bump), human status gains mode/target/errors, smc pre-open WARN → debug,
  `RuntimePaths.config_source` approved.
- [F18 — MERGED] branch `fix/observability` (88477a3) merged; gates green on merged tree (136 tests,
  fmt/clippy clean, `--features hw` guard skips). Scope-verified by orchestrator: DESIGN.md touched only
  for the approved A4 field + the additive `monitor_only` in both Appendix-B examples; cli.rs hunks are
  the status surface plus threading `globals.config` into `build_supervisor_with` (A4); the remaining
  cli.rs edits are in-file tests. Tests added: healthy daemon reports `monitor_only: false`; **induced
  degradation** (`fan1_output` chmod 0444 → curve writes fail → state/status mark monitor-only — the
  F17b regression test); human status renders mode/target/errors; schema tests cover both ids with the
  new field. README updated to match observed output (docs-vs-behavior). Product-code delta **+79 LOC**
  (budget +150). Package rebuilt 20:30.
- [F19 — CANDIDATE, hardware evidence] F16's register-echo check **also** fails on this firmware:
  after `sudo afanctl curve` (20:36:07) the journal shows `wrote=1200 read_back=2638` (stable across the
  three microsecond-spaced retries) → 3 polls → monitor-only again (fan 7200 → SMC auto). So `curve`
  still does not control on real hardware. Driver mapping confirmed from applesmc.c (the authoritative
  source, not inference): `fan1_input` = `F0Ac` (actual, RO), **`fan1_output` = `F0Tg` (target, RW)**,
  `fan1_manual` = bit 0 of `FS!` (16-bit mask), `fan1_min/max` = `F0Mn/F0Mx`. Live: auto ⇒ `F0Tg == F0Ac`
  (7200/7205); during the failures `F0Tg` read *intermediate* values (6170 → 5235 → 1866; 2638) —
  i.e. the SMC appears to **ramp** toward the commanded target and reports the in-progress value, so the
  write is likely honored and the check is a *timing* error, not a value error: K=3 retries inside one
  poll are microseconds apart, while the SMC's update cadence is ~1 s. Decisive experiment launched on
  hardware (write manual=1 → target 2000 → watch `fan1_manual`/`fan1_output`/`fan1_input` for 15 s) to
  distinguish (a) echo-with-settle-delay, (b) effect-only (F0Tg tracks the SMC's own ramp, never equals
  the command), (c) manual bit not honored on this model. Ruling F19 waits on that data — no ticket
  dispatched yet.
- [PLAN CHANGE — orchestrator] T9b review dispatched: adversarial review of everything merged *after*
  the T9 gate (F14/F16/F18 + the four rulings), because the hardware gate found a class T9 could not see
  (fixture-shaped physics assumptions). Read-only, fresh context, no hardware tests.
- [F19 — RULED (hardware-measured)] **The 20:36 failure was the stale F14 binary**, not F16: the daemon
  had been running since 20:05:50 (F14 build, tach-based verify); the F16/F18 package was installed at
  20:36 but the unit was not restarted until the SIGKILL at 20:37:11. So `read_back=2638` was
  `fan1_input` (the tach descending toward 1200) under the *old* code — F16's echo check had never been
  exercised on hardware. The 20:47 experiment then settled the semantics: with `manual=1`, writing
  `F0Tg=2000` ⇒ target reads `2000` within ≤1 s and the fan converges `5875 → 2664 → 1632 → 1940 →
  2044 → … → ~2000` (±20 rpm, small overshoot, ~5 s), and restoring auto ⇒ target `6229` (SMC's own).
  Conclusion: `fan1_output` writes **are honored**, the echo is real, and the SMC updates `F0Tg`
  **asynchronously on a ~1 s tick**. F16's defect is therefore a *timing* error: `VERIFY_ATTEMPTS = 3`
  retries run microseconds apart against a register that has not ticked yet ⇒ false "write not taken"
  ⇒ poll failures ⇒ monitor-only. RULING F19: (a) `write_speed`/`set_mode` verify inside a **settle
  window** (`ECHO_SETTLE_MS = 1500`, 10 × 150 ms) — accept as soon as the register matches; one write
  per window, no microsecond hammering; `VerifyFailed` only if the window expires; (b) keep F16's L1
  split, stall detector and dynamics; (c) `MockSmc` gains `set_echo_latency` so the stale-echo case is a
  **test** (the regression that hardware found); (d) `doctor` gains a **stale-binary check** (running
  daemon's start time older than the installed binary's mtime ⇒ WARN "restart the unit") — this exact
  confusion cost a hardware round. Ticket F19 dispatched.
- [HW DYNAMICS — measured 2026-09-14 20:47, for the mock] full-swing response ≈ 3000 rpm/s downward
  (6688→2664 in 1 s), ~5 s to settle, ±20 rpm steady-state, small overshoot (1626 → 1940 before
  settling at ~2000). Mock lag defaults should reflect this, not the idealized 1500 rpm/poll sketch.
- [T9b — REVIEW GATE ROUND 2: **FAIL**] report merged (`orchestration/REVIEW-T9b.md`, 240 lines).
  F14/F16/F18 each verified compliant *with kill-analysis* (the reviewer named the change that would
  break each ruling test), plus contract hygiene clean (no unsafe outside safety.rs, zero
  unwrap/expect/panic in product code, deps exactly the allowlist, no println outside cli.rs). Findings:
  **2 MAJOR proven live + 7 MINOR**. Orchestrator re-verified both majors in source:
  (1) `src/supervisor.rs:593-608` — the stall window resets on every *command change*, so during curve
  slew or mid-band temp oscillation it never reaches `STALL_POLLS`: a frozen fan stays invisible
  (`verified: true`, `monitor_only: false`, empty `recent_errors`) while temps oscillate — the old
  tach-counting code caught it. (2) `src/supervisor.rs:648-680` — `degrade_to_auto` sets
  `manual_armed = false` even when its own `set_mode(Auto)` **failed**, so L1's mode check switches off,
  `monitor_only` stops all writes, nothing re-attempts AUTO, and the success line "AUTO restored" is
  printed anyway: a fan left in Manual is stranded, unsupervised, until a *changed* command or process
  death. Also noted: T9b's "assumptions that only hold on fixtures" section independently flagged the
  `F0Tg`-echo latency class *before* the 20:47 measurement, and its finding 6 (no test pins the ruled
  constant values) plus finding 8 (the F18 evidence line has no test) are real test gaps.
- [F20 — RULED] DESIGN.md constants list updated with all ruled constants (F16/F19/F20). RULING F20
  closes both majors without re-adding F16's false positives: (R1) stall detector keyed to **tach
  movement** (`STALL_TACH_EPSILON_RPM = 50`) instead of command changes — a real fan's tach always
  moves when commanded elsewhere (measured: 6688 → 2001 in ~5 s, ±20 at rest); a frozen actuator's does
  not, so neither a moving command nor mid-band oscillation can buy a dead fan a fresh lease;
  (R2) failed AUTO restore stays truthful and recoverable: keep `manual_armed`, add
  `auto_restore_pending`, retry every poll (log rate-limited every `AUTO_RETRY_LOG_POLLS = 10`), only
  claim success on a verified read-back, and expose the flag additively in state.v1/status.v1;
  (R3) `apply_mode` honours the no-L2→observe invariant on the cmd-file channel too; (R4) off-target
  dwell (`OFF_TARGET_WARN_POLLS = 30`) becomes visible without degrading; (R5) README L1 + foreign-owner
  wording, constant-boundary pinning tests, evidence-line test. Ticket `orchestration/instructions/F20.md`
  rendered — **held until F19 merges** (same files: supervisor.rs/policy.rs/tests).
- [F19 — MERGED] branch `fix/settle-window` (b1a3721) merged; gates green (143 tests, fmt/clippy clean,
  `--features hw` guard skips). Orchestrator-verified: settle window implemented as
  `ECHO_SETTLE_MS = 1500 / ECHO_SETTLE_SAMPLES = 10` (+ `MODE_SETTLE_MS = 1000`, `WRITE_RETRY_MAX = 1`)
  in `policy.rs`, used by both backends in `smc.rs`; the µs retry ladder is gone; `MockSmc::set_echo_latency`
  models adoption latency and the explicit regression test `echo_latency_is_accepted_not_a_failure`
  asserts one write + no counted failure; `tests/settle_window.rs` test 1 **replays the measured hardware
  event** (fan 6688 Manual, tach lag 3000, echo latency 300 ms ⇒ converges to 1200 with
  `recent_errors == []`, `monitor_only == false`); test 2 covers "echo never adopted" ⇒ VerifyFailed +
  fallback; `doctor` gains the stale-binary classifier (+ unit test). Deviation N-F19-1: product LOC
  **+223 vs the +180 budget (24% over)** — stop-and-reported with drivers named (both mechanisms mandated
  whole + §8-mandated invariant comments); accepted per the standing budget precedent (nothing trimmed,
  no public item added beyond the ruled constants). Package rebuilt 21:10.
- [F20 — DISPATCHED] branch `fix/stall-and-auto` cut from the merged F19 tree; ticket rendered
  (`orchestration/instructions/F20.md`, 11.5 KB). This closes T9b's two MAJORs.
- [F20 — MERGED + VERIFIED] branch `fix/stall-and-auto` (7fb9381) merged; gates green (152 tests). Scope
  checked: `cli.rs` +12 is exactly the additive `auto_restore_pending` (JSON + human marker) that
  RULING F20 R2 authorises; `degrade_to_auto` now emits the success message only on `Ok` and on `Err`
  keeps `manual_armed`, sets `auto_restore_pending`, records attempts and retries every poll with a
  rate-limited log. Regressions covered by dedicated tests: `f20_moving_command_frozen_tach_stall_fires`,
  `f20_sustained_motion_healthy_fan_never_stalls`, `f20_failed_auto_restore_is_retried_truthfully`,
  `f20_l2_absent_refuses_cmd_control`, `f20_stall_fires_exactly_at_stall_polls`,
  `f20_off_target_dwell_warns_once_per_excursion`, `startup_evidence_line_names_config_source`,
  plus `tests/pin_constants.rs` (echo-tolerance boundary 49/51, stall-at-exactly-10). README L1 section
  rewritten to the real semantics + the foreign-owner and failed-AUTO paragraphs. Product LOC **+162**
  (budget +250). Deviation N-F20-1 (LOC ledger) and QUESTION N-F20-1 (R3 implemented as the
  startup-recorded `l2_absent` verdict instead of a per-poll re-check, because re-checking would break
  the frozen `MockSmc::panic_fd() == None` semantics the mandated suite depends on; behaviourally
  identical on the real backend and the cmd-file channel is now guarded) — **both ACCEPTED**. Orchestrator
  also amended DESIGN.md's Appendix-B examples with the additive field. Package rebuilt 21:30.
- [T9c — DISPATCHED] round-3 adversarial review of F19+F20 (`orchestration/instructions/T9c.md`), with an
  explicit mandate to **falsify RULING F20 R1** (jittering-but-parked fan, SMC-clamped target, healthy
  fan losing ground against a fast ramp) and to attack F19's settle window in the false-PASS direction.
- [HW GATE (d) + (e) — PASS 2026-09-14 21:48] (d) soak: unit up **1 h 10 min**, 1.966 s CPU / 4211 s wall
  (**0.05 %**, R11 budget <0.1 %), peak **2.4 MB** RSS (<5 MB), no errors. (e) the acceptance test that
  had failed three times now passes end-to-end on hardware: `curve` ⇒ `fan1_manual == 1` in ≤3 s with
  `mode: curve`, `target: 1200`, fan converging from 2005 rpm, `recent_errors: none`; SIGKILL ⇒ journal
  `startup reconcile: previous process died without restoring AUTO … restoring AUTO now` then
  `AUTO restored (fan1_manual=0, verified)`, readback `0`; `doctor`'s new stale-binary check PASSed.
  Remaining: (f) soak + `--compare` (held until T9c clears) and (g) `hold 3000`.
- [HW OBSERVATION — for the (f) judgement] with `curve` active at `t_eff` 61 °C our target is 1200 rpm
  (quiet) while the SMC's own curve wanted ~6.6k rpm: the two controllers disagree by design (ours trusts
  coretemp; the SMC's raw die sensors read ~10-15 °C hotter — e.g. 20:14 `TC0F` 63.5 C vs coretemp
  package 48-51 C). coretemp is the authoritative CPU metric (the CPU throttles on its own DTS, Tjmax
  100), so the design stands — but this is the input the user needs for the `--compare` decision, and
  `max = 86` (full speed) maps to roughly Tjmax on the SMC's own scale. Option if the user wants margin:
  lower `[thresholds].max` (e.g. 80) in `/etc/afanctl/afanctl.toml`; the file is pacman `backup=`-protected.
- [F16 — MERGED] branch `fix/write-verify` (37722cf) merged; gates green on merged tree (131 tests,
  fmt/clippy clean, `--features hw` guard skips). Verified by orchestrator in source: `SysfsSmc::write_speed`
  now reads back **`fan1_output`** against `WRITE_ECHO_TOLERANCE_RPM`; `l1_poll` splits mode-drift
  (counted) from tracking (re-assert, explicitly uncounted) and adds the `STALL_POLLS` stall detector;
  `MockSmc` gains `set_tach_lag`/`set_tach_frozen`/`set_write_stuck`. Tests present and matching the
  ruling: hardware-event repro (6170 → 1200 with `write_failures == 0`, no fallback), takeover-by-echo,
  frozen-fan stall, echo-not-taken via fault injection, mode drift, and the tracking-not-counted case.
  Product-code delta measured at **+155 LOC** (budget +250). Package rebuilt 20:22 for reinstall.
- [RULING N-F16-1 — ACCEPTED] stall detector degrades directly (loud error + AUTO + monitor-only at
  `STALL_POLLS`) instead of riding the 3-strike ladder. Accepted: a stall is already a 10-poll confirmed
  fault; stacking `WRITE_FAIL_FALLBACK` on top would delay the firmware net by ~30 s for no gain.
- [RULING N-F16-2 — ACCEPTED] a static fixture tree cannot express "write not taken" now that
  verification is the echo (the file holds what was written), so that case is fault-injected through
  `MockSmc::set_write_stuck`; the lag/Auto-mirror dynamics live in `MockSmc` (`set_tach_lag`), and the
  old `fixture_write_not_taking_fails_verify_after_k_retries` test — which encoded the F16 defect — was
  rewritten as `fixture_write_verified_by_register_echo`. Correct reading of RULING R5.
- [DEFECT F17 (observability, not safety) — found while diagnosing F16] Two gaps, both in the
  plugin-facing surface (R7/R8): (a) human `status` prints daemon/sensors/t_eff/fan/config but **not
  mode, target_rpm or recent errors**, though README promises all three; (b) **nothing exposes the
  monitor-only/degraded latch** — `afanctl.state.v1` keeps the *commanded* mode (`src/supervisor.rs`
  test asserts `state["mode"] == "curve"` while degraded), so during the F16 event the plugin/user
  would have rendered "curve" for a daemon that was doing nothing. Candidate bundle with F15 (WARN
  noise) + N-F14-1 (RuntimePaths config_source): all touch cli.rs/supervisor.rs → dispatch AFTER F16
  merges (same files, would conflict). Awaiting user go/no-go (scope, not safety).
- [T9c — REVIEW GATE ROUND 3: **FAIL** (1 MAJOR + 1 arithmetic + 5 MINOR)] report merged
  (`orchestration/REVIEW-T9c.md`, 364 lines). Method: live probes on fixture copies + read-only hardware
  sampling (20 × 1 Hz `fan1_input` deltas **0–26 rpm** — half of `STALL_TACH_EPSILON_RPM = 50`).
  **RULING F20 R1 survived its falsification mandate**: jitter-above-epsilon never stalls (proven; real
  jitter is 2–4× below the mandate's hypothetical, so the reviewer's "keep the criterion, fix
  visibility" recommendation is adopted); an SMC-clamped target degrades correctly by design; a healthy
  fan losing ground against the ramp is impossible under measured physics (slew 750 rpm/poll vs fan
  ~2000–3000 rpm/s); a frozen tach fires. F19's settle window came back clean on the false-PASS attack
  (a stale echo means the register already holds the command; clamps > 50 rpm fail the window and travel
  the counted ladder). Findings: **(1) MAJOR proven live** — the cmd-file *observe* transition drops
  ownership on a failed `set_mode(Auto)`, re-creating the T9b MAJOR-2 shape on the channel F20 R2 did not
  cover (supervision blind, fan parked in Manual, state renders healthy); **(2) arithmetic** — the settle
  window ate the watchdog margin, so a *healthy* curve poll at the legal `interval_s = 14` can push the
  ping gap to ≈15.0–15.5 s > `WatchdogSec=15` ⇒ SIGABRT crash-loop (default `interval_s = 1` is safe:
  ≈8.2 s worst gap, ~45 % margin); (3) R4's dwell WARN latches once per excursion forever; (4) two F19
  constants missing from the Appendix-A list (added by the orchestrator); (5) F19/F20 constants unpinned
  by value; (6) README L1 clause overpromises ("toward its command" vs "between polls"); (7) the
  every-poll retry and warn cadence are not test-pinned. Speculative note recorded: `doctor`'s
  stale-binary check cannot detect "package older than repo HEAD" — the reinstall+restart discipline
  covers it.
- [F21 — RULED + DISPATCHED] RULING F21: (R1) mirror F20 R2 in the observe transition — keep
  `manual_armed`, set `auto_restore_pending`, never claim an unverified release, let the every-poll retry
  finish it; (R2) restore the watchdog margin structurally — ping twice per poll (start + end of
  `step_once`) **and** lower `config::MAX_INTERVAL_S` 14 → 12 with the worst-case reasoning documented;
  (R3) make the off-target dwell WARN repeat every `OFF_TARGET_WARN_POLLS` while the excursion persists,
  documenting the movement-vs-direction trade-off (the stall criterion itself stays as ruled); (R4) pin
  the F19/F20 constants and the two cadences by value and behaviour. Ticket
  `orchestration/instructions/F21.md` dispatched on `fix/observe-release` (glm-5.3-flash high); a narrow
  T9d review of F21 follows before gate (f) is declared clean.
- [F21 — MERGED + VERIFIED] branch `fix/observe-release` (1c8d033) merged; gates green (160 tests). Verified
  in source: the observe branch now keeps `manual_armed`, sets `auto_restore_pending`/`attempts` and calls
  `fail_write` for the truthful record while still moving `mode` to Observe (the requested intent), so the
  every-poll retry completes the release; `ping_watchdog()` is called at the start **and** the end of
  `step_once`; `MAX_INTERVAL_S = 12`; the dwell WARN repeats (no latch) and resets only on convergence.
  Tests: `f21_failed_observe_release_keeps_ownership_and_retries`,
  `f21_auto_restore_retry_genuinely_attempts_every_poll`,
  `f21_watchdog_pings_at_start_and_end_of_each_poll`, `f21_long_poll_still_pings_at_its_start`,
  `f21_constants_are_pinned_by_value`, `f21_max_interval_cap_12_legal_and_14_rejected`,
  `f20_off_target_dwell_warns_at_window_and_resets_on_convergence`,
  `f21_sub_epsilon_jitter_while_off_target_is_motionless_and_stalls`,
  `f21_super_epsilon_jitter_never_stalls_but_warns_repeatedly`. Package rebuilt 22:14.
- [F22 — RULED + MERGED] F21's implementer flagged `N-F21-1` honestly instead of reaching into a file it
  did not own: with two pings per poll, `watchdog_pings` advances 2×, so the plugin-facing
  `daemon.uptime_s` (pings × interval) reported double. RULING F22 = publish a real additive `polls`
  counter (once per completed poll; `watchdog_pings` keeps its L3 meaning) and compute `uptime_s` from it,
  with a documented fallback for a state file from an older build; the test that *pinned the wrong
  arithmetic* was corrected rather than worked around. Merged (aa93070) — gates green (162 tests), product
  delta small, `tests/schema.rs` + README updated. DESIGN.md Appendix B example now carries `polls`.
  Orchestrator also amended Appendix A's `step_once` comment for the two-ping order (`N-F21-2`).
- [T9d — DISPATCHED] final review round (F21+F22) with a mandatory **CLOSE / DO-NOT-CLOSE** judgement on
  the review gate, a re-trace of every path that clears `manual_armed`, a recomputation of the watchdog
  margin at `interval_s = 12`, and a last sweep for remaining fixture-shaped assumptions (with what the
  hardware gate can and cannot cover). Package rebuilt 22:24.
- [T9d — REVIEW GATE ROUND 4 (FINAL): **PASS / review gate CLOSE**] report merged
  (`orchestration/REVIEW-T9d.md`, 480 lines). The agent's session was killed (exit 140) *after* it had
  written the complete report but before committing; the orchestrator committed the untouched artifact
  on its branch and merged it (salvage recorded here for honesty — the content is the reviewer's).
  Verdict: **0 MAJOR, 0 MINOR, 3 INFORMATIONAL.** All F21/F22 rulings match and their tests genuinely
  pin them (kill-analysis); every path that clears `manual_armed` was re-traced; the watchdog
  arithmetic was recomputed from source; the F16/F19/F20 semantics are untouched (`src/smc.rs` is
  byte-identical in the reviewed range). Informational: (1) the `MAX_INTERVAL_S` doc comment cites the
  single-window worst case (≈2.7 s) instead of the true in-poll ceiling (≈5 s common, ≈10 s if all four
  write paths fail) — the value 12 is still safe because the two pings bound the observed gap by
  `max(interval, poll_work)`; (2) the README should say a plugin must read `auto_restore_pending` (the
  `mode` may read `observe` while the fan is still Manual and the release is pending); (3) a cosmetic
  warn-cadence boundary coincidence, explicitly "no direction". T9d's own recommendation was to fix (1)
  and (2) as docs and to **decline** the optional `verified: false`-while-pending semantics change —
  adopted. F23 (doc-only) dispatched to close (1) and (2).
