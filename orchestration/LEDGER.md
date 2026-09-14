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
