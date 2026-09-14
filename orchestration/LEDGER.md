# afanctl orchestration ledger — orchestrator-owned; subagents read-only

## Index
| id | title | model | branch | status | dispatches | bounces | timeouts | merged (sha) |
|----|-------|-------|--------|--------|------------|---------|----------|--------------|
| S0 | bootstrap | glm-5.3 (orchestrator) | — | MERGED | 0 | 0 | 0 | (initial commit) |
| T0 | scaffold+contracts | glm-5.3-flash/high | t0-scaffold | DISPATCHED | 1 | 0 | 0 | — |
| T1 | config | deepseek-v4.1-flash | t1-config | DISPATCHED | 1 | 0 | 0 | — |
| T2 | policy | glm-5.3-flash/high | t2-policy | DISPATCHED | 1 | 0 | 0 | — |
| T3 | smc | glm-5.3-flash/high | t3-smc | DISPATCHED | 1 | 0 | 0 | — |
| T4 | safety+notify | glm-5.3-flash/high | t4-safety | DISPATCHED | 1 | 0 | 0 | — |
| T5 | supervisor | glm-5.3-flash/high | t5-supervisor | PENDING | 0 | 0 | 0 | — |
| T6 | cli+main (a: parser; b: wiring) | deepseek-v4.1-flash | t6-cli | PENDING | 0 | 0 | 0 | — |
| T7 | doctor | deepseek-v4.1-flash | t7-doctor | PENDING | 0 | 0 | 0 | — |
| T8 | integration+packaging | deepseek-v4.1-flash | t8-integration | PENDING | 0 | 0 | 0 | — |
| T9 | review gate | glm-5.3-flash/high | — (read-only) | PENDING | 0 | 0 | 0 | — |

<!-- statuses: PENDING · DISPATCHED · VERIFYING · BOUNCED(n) · MERGED · BLOCKED · ESCALATED -->

## Events (append-only, newest last)
- [2026-09-14] S0: smokes passed (deepseek-v4.1-flash OPENCODE_SMOKE_OK; --auto write path ok). rust 1.98.1 installed via pkexec. repo initialized (identity pkrc267@gmail.com). PRD.md + PLAN.md + orchestration/** committed.
