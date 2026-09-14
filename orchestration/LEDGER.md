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
| T5 | supervisor | glm-5.3-flash/high | t5-supervisor | PENDING | 0 | 0 | 0 | — |
| T6 | cli+main (a: parser; b: wiring) | deepseek-v4.1-flash | t6-cli | MERGED(a) | 1 | 0 | 0 | 815d0fa |
| T7 | doctor | deepseek-v4.1-flash | t7-doctor | PENDING | 0 | 0 | 0 | — |
| T8 | integration+packaging | deepseek-v4.1-flash | t8-integration | PENDING | 0 | 0 | 0 | — |
| T9 | review gate | glm-5.3-flash/high | — (read-only) | PENDING | 0 | 0 | 0 | — |

<!-- statuses: PENDING · DISPATCHED · VERIFYING · BOUNCED(n) · MERGED · BLOCKED · ESCALATED -->

## Events (append-only, newest last)
- [2026-09-14] S0: smokes passed (deepseek-v4.1-flash OPENCODE_SMOKE_OK; --auto write path ok). rust 1.98.1 installed via pkexec. repo initialized (identity pkrc267@gmail.com). PRD.md + PLAN.md + orchestration/** committed.
- [wave 1] T1/T2/T3/T4/T6a all verified first-pass (gates green, ownership clean, porcelain empty).
  Merged T1→T2→T3→T4→T6a (rebases union-resolved on append-only D/Q files only).
  Adjudications: D-T6-1 ACCEPTED (doctor::run +json, DESIGN.md amended, T6 fixed on branch pre-merge);
  T3-N2 intrinsic layout hook blessed; T1 Q-T1-1 (strict missing-key refusal) + T6 Q-T6-1
  (AFANCTL_RUNTIME_DIR) now spec. LOC overages config.rs/smc.rs/cli.rs noted, within stop-and-report rule.

- [A1 pending] T5 next.
