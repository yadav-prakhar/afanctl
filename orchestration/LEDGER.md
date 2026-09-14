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
| T9 | review gate | glm-5.3-flash/high | t9-review | REPORTED | 1 | 0 | 0 | (report merged) |

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
