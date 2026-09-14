# PLAN — `afanctl` build orchestration

**Version:** 1.0 · **Date:** 2026-09-14 · **Status:** Ready to execute
**Source of truth:** `PRD.md` (self-contained, evidence re-verified 2026-09-14). Where this plan and the PRD could disagree, **the PRD wins**; this plan adds *how* the work is governed, not *what* is built.
**Reader:** the Orchestrator (§3) — a fresh Hermes session on glm-5.3 (maximum thinking) or a human operator following §3 mechanically. No conversation history is needed: everything is here or in `PRD.md` (same directory, committed to the repo in S0).

**Crew (all via provider `opencode-go`, dispatched with the `opencode` CLI):**

| Role | Model | Variant | Assigned to |
|---|---|---|---|
| Orchestrator (governance, §3) | glm-5.3 (the Hermes planning session itself) | max thinking | S0, merges, audits, adjudication, handoff |
| Safety-critical implementer | `opencode-go/glm-5.3-flash` | `--variant high` | T0, T2, T3, T4, T5, T9 |
| Mechanical implementer | `opencode-go/deepseek-v4.1-flash` | default | T1, T6, T7, T8 |

Routing follows PRD §11: stateful / `unsafe` / safety-critical → glm-5.3-flash (high); exactly-specified mechanical surfaces → deepseek-v4.1-flash. Both model IDs verified available and responding on this box today (§0).

---

## 0. Verified execution environment (checked live 2026-09-14, this session)

| Fact | Value | Consequence |
|---|---|---|
| opencode CLI | 1.18.30 at `~/.local/share/mise/installs/opencode/latest/opencode` (also on PATH) | dispatch commands in §4.4 run as written |
| opencode auth | OpenCode Go + OpenRouter credentials present | no login step needed |
| `opencode-go/glm-5.3-flash` | listed, smoke-answered `OPENCODE_SMOKE_OK` | crew model verified |
| `opencode-go/deepseek-v4.1-flash` | listed in `opencode models` | re-smoke in S0 before first deepseek dispatch |
| Dispatch flags | `run`, `--model provider/model`, `--variant high`, `--title`, `--auto`, `-f` all exist in `run --help` | exact commands in §4.4 |
| git | 2.55.0 present | branches/worktrees as spec'd |
| Rust toolchain | **NOT installed** (`pacman -Q rust` → not found; no `cargo`/`rustc` on PATH) | S0 installs `extra/rust` — matches PRD §2.1 |
| Elevation | `sudo` needs a password with no askpass; working pattern on this box is `pkexec --disable-internal-agent <cmd>` (graphical polkit prompt; Omarchy's quickshell runs the agent) | S0 uses pkexec; **the user must be present to approve the prompt** |
| Workspace | `/home/prakhar/Work/tries/2026-09-14-a1708-fanctl/` contains only `PRD.md`; not yet a git repo | S0 runs `git init` and commits `PRD.md` + `PLAN.md` |

---

## 1. Schedule & task graph

Merge order is strict; dispatch within a wave is parallel (one git worktree + one opencode session per task — never share a working directory between sessions).

```
S0 bootstrap (orchestrator, no dispatch)
  │
  ▼
T0 scaffold+contracts (glm-5.3-flash/high)  ── merge → main  (DESIGN.md now exists)
  │
  ├──────────────────────────────► T6a cli+main, parser & formats (deepseek) ─┐
  ▼                                                                        │
WAVE 1 — parallel dispatches:                                              │
  T1 config (deepseek) · T2 policy (glm/high) · T3 smc (glm/high)          │
  T4 safety+notify (glm/high — codes against frozen trait stubs)           │
  merge order: T1 → T2 → T3 → T4 → T6a (any order among T1/T2/T3;         │
  T4 strictly after T3; T6a whenever its gates pass)                       │
  │                                                                        │
  ▼                                                                        │
T5 supervisor (glm/high) ──► AUDIT A1 (orchestrator; PRD §11.4: after T5)
  │
  ▼
T6b final wiring (deepseek, short re-dispatch from T6a's card)
  │
  ▼
T7 doctor (deepseek)
  │
  ▼
T8 integration+packaging (deepseek)
  │
  ▼
AUDIT A2 (orchestrator)
  │
  ▼
T9 review gate (glm-5.3-flash/high, FRESH context) ── orchestrator verifies its findings ── fix tickets loop
  │
  ▼
HANDOFF — supervised hardware gate §9.3 + `makepkg -si` §9.4 (USER-present; orchestrator never runs it)
```

Wall-clock guidance (single parallel wave counted once): S0 ≈ 10 min + install; T0 15–30 min; Wave 1 30–60 min; T5 30–45 min; T6b 10–15 min; T7 30 min; T8 45–60 min; T9 30–60 min; fix loop variable. Budget roughly half a day to a day end-to-end including one fix loop.

**Optional acceleration (orchestrator's discretion, default OFF):** T7 and T8 touch disjoint files; they may be dispatched in parallel once T6b is merged, provided T8's integration tests that reference `doctor` behaviors (if any emerge) are deferred to the fix loop. The PRD's graph (§11.1) sequences T7 → T8; deviating is a ledger-noted decision.

---

## 2. Task cards

Each card below is the **operative summary**; the spec authority is the PRD (§6 requirement, §7 Appendix A signatures, §11.2 card text). The rendered instruction (Appendix P2) always tells the agent to read `DESIGN.md` first and then the PRD sections this card references. LOC budgets are from PRD §7; >20% over = stop and report.

### S0 — bootstrap (orchestrator-executed, not dispatched)

**Do:**
1. Re-verify dispatch readiness: `opencode auth list` shows OpenCode Go; smoke both crew models (`opencode run 'Respond with exactly: OPENCODE_SMOKE_OK' --model opencode-go/deepseek-v4.1-flash`, same for glm-5.3-flash); smoke the permission path with a trivial write task using `--auto` (create+delete a scratch file) so the first real dispatch doesn't hang on a permission prompt.
2. Install Rust: `pkexec --disable-internal-agent pacman -S --needed rust` (user approves the polkit prompt), then verify `cargo --version` and `rustc --version` (expect 1.98.x per PRD §8).
3. `git init` the workspace; confirm `git config user.email` is set (set a local identity if not). Commit `PRD.md` and `PLAN.md` on `main`.
4. Create orchestrator-private scaffolding (committed): `orchestration/LEDGER.md` (Appendix P4), `orchestration/instructions/` (empty), and a `.gitignore` containing `orchestration/logs/` and `target/`.
5. Seed the ledger index with all tasks PENDING.

**Done when:** both smokes pass, `cargo --version` works, `main` holds PRD+PLAN+orchestration skeleton, ledger initialized. If the user is absent for the polkit prompt, STOP and tell them — nothing else can proceed.

### T0 — scaffold + contracts · `glm-5.3-flash --variant high` · Wave 0 · branch `t0-scaffold`

- **Goal:** create the crate with every PRD Appendix-A signature present as compiling stubs; each stub body `unimplemented!("owned by T<n>")`, tagged with its owning task. Crate compiles, all four §8 gates pass, any called stub fails loudly.
- **Files owned:** `Cargo.toml`, all `src/*.rs` stubs, `DESIGN.md` (verbatim copy of PRD §7+§8 — this becomes the in-repo binding contract), `DEVIATIONS.md`, `QUESTIONS.md`, `tests/` skeleton, `tests/fixtures/sysfs/` skeleton mirroring PRD §2.1 exactly (hwmon4 under coretemp.0 with Package id 0/Core 0/Core 1, attribute-less husk hwmon3 under applesmc, `fan1_*` files with 1200/7200), `check.sh` (runs the four gates).
- **Spec:** PRD §7 (layout + Appendix A signatures), §8, §11.2-T0. Dependency allowlist exactly R11: `serde`, `toml`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber`, `libc` (plus dev-deps as needed for tests only if already in the allowlist spirit — any addition goes to DEVIATIONS).
- **Tests:** none functional yet; gates green is the test.
- **Done:** gates green in clean checkout; every signature byte-identical to Appendix A; every stub tagged with its owner; `DESIGN.md` diffed against PRD §7+§8 by the orchestrator before merge.
- **Orchestrator review emphasis:** signature fidelity (diff against PRD Appendix A mechanically — do not eyeball).

### T1 — config · `deepseek-v4.1-flash` · Wave 1 · branch `t1-config`

- **Goal:** implement R6 + Appendix A `Config`/`ConfigError`/`ResolvedConfig` exactly.
- **Files owned:** `src/config.rs` (+ unit tests), `packaging/afanctl.toml.default`.
- **Spec:** PRD §6.R6, §7 Appendix A (config section), §11.2-T1.
- **Tests:** defaults; every validation rejection with key+fix in the message (`high >= max`, `max > 95`, `min_rpm >= max_rpm`, `interval_s < 1`, non-integer temps, the H3 landmine `high == max`); unknown-key warning collection; missing file → defaults; clamping against hw tuple.
- **Done:** gates + tests green.
- **Orchestrator review emphasis:** error messages name key AND fix (checklist item 5); no config key beyond the five.

### T2 — policy · `glm-5.3-flash --variant high` · Wave 1 · branch `t2-policy`

- **Goal:** implement R2 + `Controller` exactly — pure, no I/O/clock/threads, absolute target curve recomputed every poll, hysteresis, slew limiter, overshoot guard, sensor-loss streaks, named constants.
- **Files owned:** `src/policy.rs` (+ unit tests, `MilliC` lives here per T0's decision), `tests/policy_traces.rs`.
- **Spec:** PRD §6.R2, §7 Appendix A (policy section), §11.2-T2.
- **Tests:** the seven R10 trace families as data tables (plateau ×50 → f(80); hot-core {92,60,88} acts on 92; descending approach, no ratchet; hysteresis no-flap sweep; sensor-loss streak → `ReturnToAuto`; overshoot guard; hold-with-hot-core escalates); linear-endpoint exactness at `high`/`max` boundaries.
- **Done:** gates + all traces green.
- **Orchestrator review emphasis:** checklist item 2/3 analog — every decision path is `Observe`/`SetSpeed`/`EscalateMax`/`ReturnToAuto`, never silence; interim sensor loss writes nothing new.

### T3 — smc · `glm-5.3-flash --variant high` · Wave 1 · branch `t3-smc`

- **Goal:** implement R1 + `SysfsSmc`/`MockSmc`: discovery walks (never hardcoded hwmonN), read-back-verify invariant with K=3 retries, clamping to hw range, outlier rejection (<0 or >120 °C), `panic_fd`.
- **Files owned:** `src/smc.rs` (+ unit tests), `tests/fixtures/sysfs/**` (completing T0's skeleton with behavioral fixture variants).
- **Spec:** PRD §6.R1, §7 Appendix A (smc section), §11.2-T3, §10 Q4/Q5.
- **Tests (fixtures):** round-trip verify; write-that-doesn't-take → `VerifyFailed` after K retries; failed/short reads; mode flip; outlier rejection; layout-change detection hook (for doctor).
- **Done:** gates + tests green; **no path strings outside this file** (orchestrator greps `src/` for `/sys/` — only smc.rs may hit).
- **Orchestrator review emphasis:** the R1 invariant — no logical state from unverified writes; grep the path confinement.

### T4 — safety + notify · `glm-5.3-flash --variant high` · Wave 1 (merge after T3) · branch `t4-safety`

- **Goal:** implement L2 exactly as R4/Appendix A — pre-opened fd, panic hook, raw `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers, exactly one async-signal-safe `write(fd, b"0")`; `arm_test_panic`. `notify.rs`: hand-rolled sd_notify via `$NOTIFY_SOCKET` datagram, no-op-false when unset.
- **Files owned:** `src/safety.rs`, `src/notify.rs` (+ unit tests where testable).
- **Spec:** PRD §6.R4 (L2), §7 Appendix A (safety/notify sections), §11.2-T4.
- **Tests:** testable pieces (notify socket path handling, fd validity); the signal path itself is proven in T8's fixture `selftest-panic` test and the hw gate.
- **Done:** gates green; `unsafe` confined to safety.rs with a `// SAFETY:` comment naming the async-signal-safety argument on every block.
- **Orchestrator review emphasis:** this is the trickiest ~20 lines in the project (per the source debate) — review with checklist items 1–3 at full attention; no allocation/formatting/locks/path construction inside handlers.

### T5 — supervisor · `glm-5.3-flash --variant high` · Wave 2 (after T1+T2+T3+T4 merged) · branch `t5-supervisor`

- **Goal:** implement R3 modes + the poll loop in EXACTLY this order: read cmd file → validate/apply mode → read sensors → controller step → act via smc (verify) → L1 re-assert → write `state.json` → watchdog ping → sleep. `step_once` returns `StepReport`; L1 counters → fallback-to-AUTO + monitor-only degradation after `WRITE_FAIL_FALLBACK`.
- **Files owned:** `src/supervisor.rs` (+ unit tests).
- **Spec:** PRD §6.R3, R4 (L1), R7, R8, §7 Appendix A (supervisor section), §11.2-T5.
- **Tests (MockSmc):** mode transitions via cmd file; invalid cmd ignored+logged; L1 drift re-assert; fallback path; hold clamped; state file written correctly.
- **Done:** gates + tests green.
- **Orchestrator review emphasis:** checklist items 2–3 (every write verified; every failure → AUTO / loud log); cmd file never bypasses the smc write-verify path.

### T6 — cli + main · `deepseek-v4.1-flash` · Wave 1 (phase a) + re-dispatch after T5 (phase b) · branch `t6-cli`

- **Goal (phase a):** hand-rolled arg parser (~80 LOC) with the exact R5 grammar; verbs dispatch against the frozen signatures; exit codes 0/1/2; unknown flag → usage on stderr + exit 2; human output formats per Appendices B/C; `main.rs` wiring only. Gates must pass against T0's compiling stubs.
- **Goal (phase b, short re-dispatch after T5 merges):** wire `daemon`/`once`/`hold` to the real Supervisor end-to-end; adjust output details discovered only against real `StepReport` data; verify `status` merges config + state file + direct sysfs reads (R7).
- **Files owned:** `src/cli.rs`, `src/main.rs`.
- **Spec:** PRD §6.R5, R7, R11 (exit codes), §7 Appendices B/C, §11.2-T6.
- **Tests:** parser table (every verb, every error case, exit codes); `--version`; phase b adds nothing new unless wiring exposed a gap (gap → fix + test).
- **Done:** gates + parser tests green; no logic in main.rs beyond wiring.
- **Orchestrator review emphasis:** exit-code discipline (mbpfan's exit-0 disease stays dead); no `println!` outside cli.rs output formatting.

### T7 — doctor · `deepseek-v4.1-flash` · Wave 3 (after T6b) · branch `t7-doctor`

- **Goal:** implement R5's doctor check list (all read-only unless `--roundtrip`) + `--compare N` (sample t_eff + SMC rpm in observe, simulate the curve over the same trace, paired table + divergence stats, Appendix C format, exit 1 on any FAIL). Uses smc's layout-change hook (Q4).
- **Files owned:** `src/doctor.rs` (+ unit tests).
- **Spec:** PRD §6.R5 (doctor checks list), §7 Appendix C, §11.2-T7.
- **Tests:** each check against fixture trees (healthy, missing driver, unwritable, bad config, changed layout); compare math on a recorded trace.
- **Done:** gates + tests green.
- **Orchestrator review emphasis:** `--roundtrip` is the ONLY write doctor ever does, behind its confirm flag — verify no other write path exists in doctor.rs.

### T8 — integration + packaging · `deepseek-v4.1-flash` · Wave 4 (after T7) · branch `t8-integration`

- **Goal:** integration tests spawning the real binary with `--sysfs-root` fixtures; packaging per R9/Appendix D + polkit rule per Q8; README.
- **Files owned:** `tests/integration.rs`, `tests/schema.rs`, `packaging/afanctl.service`, `packaging/PKGBUILD`, `packaging/` polkit rule, `README.md`.
- **Spec:** PRD §6.R9, R10 (integration/schema), §7 Appendix D, §10 Q8, §11.2-T8.
- **Tests:** `once` decision output; `hold` → cmd file → spawned daemon applies within one poll; `selftest-panic` exits nonzero AND fixture `fan1_manual == 0`; L1 re-assert on induced drift; `status --json` validates against `afanctl.status.v1`.
- **Done:** gates + integration green; `makepkg` runs (full `makepkg -si` is user-gated, §9.4); PKGBUILD installs binary + unit + `afanctl.toml.default` + polkit rule with `backup=('etc/afanctl/afanctl.toml')`.
- **Orchestrator review emphasis:** checklist item 4 — no test writes real sysfs (grep tests for `--sysfs-root` usage and `AFANCTL_HWTEST` gating); systemd unit byte-compare against Appendix D.

### T9 — review gate · `glm-5.3-flash --variant high`, FRESH context · Wave 5 (after T8 + Audit A2) · branch none (read-only)

- **Goal:** adversarial review of the whole tree: every §6 requirement met; every §9.1–9.5 criterion evidenced; each mbpfan defect class (C1 C2 H1 H2 H3 H4 H5 H6) mapped to the test that proves it dead; conventions audit (unsafe/allowlist/LOC/newtypes); README accuracy. Output PASS or a prioritized defect list.
- **Files owned:** none — it may open fix tickets, never fix directly.
- **Spec:** PRD §9, §11.2-T9, §11.5 (full checklist incl. item 6).
- **Done:** report delivered; the orchestrator then mechanically re-verifies every claimed defect (a defect that doesn't reproduce is dropped, noted in ledger) and dispatches fix tickets (same conventions, same gates, files owned = the fix's target files).
- **Rationale for fresh-context dispatch:** the orchestrator is steeped in its own assumptions; a cold reader catches what a warm one cannot. Fallback (orchestrator's call): run the review itself if dispatch fails twice, and note the deviation in the ledger.

---

## 3. Orchestrator runbook (governance)

The Orchestrator is the PRD's planner role made explicit: **glm-5.3, maximum thinking, running as the Hermes session that executes this plan** (a human following this section mechanically is an acceptable substitute). It governs; it does not implement.

### 3.1 Authority

**The Orchestrator MAY:** dispatch and re-dispatch subagents (§3.4); run gates, greps, audits anywhere; create/delete branches and worktrees; rebase, merge, and revert task branches; edit `orchestration/**`, `PLAN.md`, and — as the sole governance exception — amend `DESIGN.md` after approving a DEVIATION (ledger-noted, DEVIATIONS entry marked APPROVED); answer QUESTIONS.md entries by ruling from the PRD; open fix tickets; halt any session.

**The Orchestrator MUST NOT:** write or fix product code itself (`src/`, `tests/`, `packaging/` are subagent territory — the only exception is revert operations); run the daemon or any test against real hardware (§9.3/§9.4 are the user's, always); use `sudo`/`pkexec` beyond S0's rust install; merge anything whose gates it has not personally re-run; trust a subagent's self-report (§3.5); approve a DEVIATION that changes a frozen Appendix-A signature without re-dispatching every in-flight affected task with the ruling.

### 3.2 Governing invariants (the six levers, operationalized)

Each lever becomes a mechanical check the orchestrator runs at verify time — no taste, no judgment calls:

| # | Lever (PRD §11) | Mechanical check |
|---|---|---|
| 1 | Frozen signatures | `diff` of public items vs `DESIGN.md` Appendix A (T0 merge does this byte-exact; later merges: any public-item change MUST have an APPROVED DEVIATION entry) |
| 2 | Conventions pasted verbatim | Every rendered instruction contains Appendix P1 (PRD §8) byte-identical — orchestrator renders from this file, never re-types |
| 3 | Strict file ownership | `git diff --name-only main..<branch>` ⊆ card's owned files + {`DEVIATIONS.md`, `QUESTIONS.md`} (append-only) — anything else is an automatic bounce |
| 4 | Mechanical gates | The four §8 gate commands re-run by the orchestrator in the task worktree with `git status --porcelain` empty |
| 5 | Per-merge review | Checklist items 1–5 (Appendix P3) applied per merge; item 6 at T9 |
| 6 | DEVIATIONS/QUESTIONS as the only cross-agent channel | grep the branch diff: any contract change NOT logged in DEVIATIONS.md = automatic bounce, even if the change is good |

### 3.3 Task lifecycle state machine

```
PENDING ──dispatch──► DISPATCHED ──agent reports done──► VERIFYING ──gates+checklist green──► MERGED
                        │                                │
                        │ timeout/stuck                  │ red/fail
                        ▼                                ▼
                   re-dispatch (fresh, ≤2 timeouts)   BOUNCED(n) ──re-dispatch with failing output──► DISPATCHED
                                                         │ n=2 and still red
                                                         ▼
                                                      ESCALATED (user)
Any moment: real-/sys write attempt or system mutation outside worktree ──► HALT: kill session, revert branch, ESCALATED (user)
```

Exit criteria: **MERGED** = gates green + checklist 1–5 pass + ownership clean + deviations adjudicated, merged to main, gates re-run on merged main. **ESCALATED** = the orchestrator stops and reports to the user with the transcript; it never absorbs a third bounce.

### 3.4 Dispatch protocol (exact commands)

One dispatch = one fresh opencode session in its own worktree. Never `--continue` a completed task; bounces are fresh sessions whose instruction carries the failing output.

```bash
# 1. Render the instruction (repo root, on main) from §2 card + Appendix P1 + Appendix P2,
#    plus any RULINGS preamble (§3.6). Write to orchestration/instructions/T<n>.md and commit.
git add orchestration/ && git commit -m "orch: instruction T<n> (rev <r>)"

# 2. Cut an isolated worktree for the task (branch cut from CURRENT main = deps merged).
git worktree add ../afanctl-wt/T<n> -b t<n>-<slug>

# 3. Dispatch in the background from the worktree; log everything.
cd ../afanctl-wt/T<n> && opencode run --auto \
  --model opencode-go/<model-id> [--variant high] \
  --title "afanctl T<n> <slug>" \
  'Read orchestration/instructions/T<n>.md at the repo root and execute it exactly — it is your
   complete specification. DESIGN.md is binding. PRD.md sections it cites are spec authority.
   Commit all work on the current branch and leave git status clean. Do not merge; do not
   touch main.' 2>&1 | tee orchestration/logs/T<n>-<attempt>.log
```

- `--auto` is required so non-interactive runs don't hang on permission prompts; it is safe **only because** the instruction's RULES block (Appendix P2) forbids sudo/pkexec/package installs/anything outside the worktree, and §3.5 verifies ownership mechanically. Never dispatch without those rules in the instruction.
- Run the dispatch in the background (`terminal background=true notify=true` for a Hermes orchestrator; a plain terminal for a human), poll the log every ~2–3 min. **Hard timeout 45 min** → kill, re-dispatch fresh. Two consecutive timeouts → ESCALATED. A timeout is not a quality bounce (separate counters).
- Tag every ledger event with the `--title` used, so sessions are auditable via `opencode session list`.

### 3.5 Verification protocol (never trust self-reports)

Subagent summaries are claims, not facts. After every "done" report, in the task worktree:

```bash
cd ../afanctl-wt/T<n>
git status --porcelain          # MUST be empty (no untracked/modified files — untracked .rs
                                # files would compile in and fake a green gate)
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw        # MUST skip cleanly (proves the hw guard works, no applesmc here)
git diff --name-only main...    # ownership check vs the card (lever 3)
git log main.. --oneline        # sane history, no merge commits into main
```

Then the per-merge checklist items 1–5 (Appendix P3) applied by reading the actual diff. Any red, any ownership violation, any unlogged contract change → bounce (§3.3). Only then merge (§3.7).

### 3.6 Adjudication: DEVIATIONS and QUESTIONS

Read both files at every verify. They are the only channel by which contract changes or ambiguities travel (lever 6).

**DEVIATIONS (proposed contract changes):**
1. Internal-only change (private items, test helpers, internal refactors), tests green → **accept**, note in ledger.
2. Frozen Appendix-A signature change → orchestrator rules from the PRD's intent:
   - **Accept** (rare, must make the contract *more* faithful to R1–R11): amend `DESIGN.md` (the sole governance exception), mark the DEVIATIONS entry APPROVED, and **re-dispatch every in-flight task whose card touches the changed item with a RULINGS preamble**; queued tasks get the ruling rendered into their instruction automatically.
   - **Reject** (default): bounce with the ruling; the agent implements the frozen contract.
3. A contract change found in the diff with NO DEVIATIONS entry → automatic bounce even if the change is good (the protocol itself is what's being enforced).

**QUESTIONS (ambiguities):** the orchestrator answers from the PRD (it has the whole document); append the ruling under the question as `RULING (orchestrator, <date>): …`, propagate as above. Never let two agents invent different answers to the same question — if a question cannot be answered from the PRD, ESCALATE to the user; do not invent spec.

**Shared-file discipline:** `DEVIATIONS.md`/`QUESTIONS.md` are append-only, each entry headed `## [T<n>] <title> (<date>)`. Parallel-wave rebase conflicts on these files resolve trivially under that discipline; the orchestrator performs the rebase.

### 3.7 Merge protocol

Strict order per §1. For each task, in the main worktree:

```bash
git -C ../afanctl-wt/T<n> rebase main        # resolve only shared-file appends (§3.6)
# re-run §3.5 gates in the worktree post-rebase (rebase can break as much as it fixes)
cd <repo-root> && git merge --no-ff t<n>-<slug> -m "merge T<n> <title> (wave <w>)"
# run the four gates ON MERGED MAIN — red here means bounce the merge:
git revert -m 1 <merge-sha>   # and bounce the task with the failing output
git worktree remove ../afanctl-wt/T<n> && git branch -d t<n>-<slug>
```

**Audits (orchestrator-run, after T5 = A1 and after T8 = A2, per PRD §11.4):**

```bash
grep -rn "unsafe" src/                 # hits ONLY in safety.rs, each with // SAFETY:
grep -rn "/sys/" src/                  # hits ONLY in smc.rs (path confinement, R1)
grep -rEn "unwrap\(|expect\(" src/     # only allowlisted sites (tests/main.rs/safety.rs)
# dependency check: Cargo.toml [dependencies] == R11 allowlist exactly
wc -l src/*.rs                          # vs §7 budgets, ±20% rule
grep -rln "AFANCTL_HWTEST" src/ tests/  # only in hw-gated test files
grep -rn "sysfs-root\|sysfs_root" tests/integration.rs   # integration really uses fixtures
```

Any audit red → fix ticket to the owning task's model (same conventions, same gates). A1/A2 results go in the ledger verbatim.

### 3.8 Failure handling

- **Bounce:** re-dispatch fresh with the instruction augmented: `BOUNCE #<n>: the previous attempt failed verification thus: <gate output / checklist item / ownership diff>. Fix and complete the card.` Max 2 bounces, then ESCALATED with the transcript.
- **Timeout/stuck:** kill, fresh re-dispatch (≤2), then ESCALATED. Inspect `orchestration/logs/` before killing to distinguish stuck from slow.
- **HALT conditions (kill session, revert branch, ESCALATED immediately):** any attempted write to real `/sys`; any use of sudo/pkexec/package installation by an agent; any file mutation outside its worktree; any attempt to merge into main by an agent.
- **Model/provider outage:** re-try once after 5 min; if the provider is down for a wave, mark tasks BLOCKED, tell the user, resume when healthy (ledger records the gap).

### 3.9 Ledger (`orchestration/LEDGER.md` — orchestrator-owned, subagents read-only)

Format in Appendix P4. Rules: every state transition gets a timestamped event line; gate outputs pasted trimmed; every DEVIATIONS/QUESTIONS ruling recorded with its propagation list; audits pasted verbatim; the final report (§3.10) is written into it before handoff. The ledger is the build's flight recorder — a stranger reading only PRD + PLAN + LEDGER must be able to reconstruct every decision.

### 3.10 Reporting & handoff

**Per merge:** one line to the user — `T<n> MERGED (gates green, checklist 1–5 pass, <bounces> bounces, <deviations adjudicated>)`. **On ESCALATE/HALT:** immediately, with the transcript path.

**End state:** when T9 (plus fix loop) closes with PASS, the orchestrator: (1) writes the final report into the ledger — per-task status, total dispatches/bounces, accepted deviations, rulings, final gate output, LOC table vs budget; (2) prints the **supervised hardware gate checklist** (Appendix P5) to the user and stops. The orchestrator never executes §9.3/§9.4 — they require the user present at the keyboard of a machine whose fan they care about, and that is precisely the boundary this whole plan exists to keep.

---

## Appendix P1 — Conventions block (PRD §8, VERBATIM — paste into every instruction)

> This is a byte-faithful copy of PRD §8 as of plan time. If PRD §8 and this copy ever diverge, the PRD wins and the orchestrator re-renders from the PRD.

**Language & toolchain**
- Rust edition 2021; rustc 1.98 (Arch `extra/rust`; install via `sudo pacman -S rust` — or `pkexec --disable-internal-agent pacman -S rust` on this box).
- No async. No threads beyond the main loop (single-threaded design; `Send` bound on `Smc` is for the trait's future, not for spawning).
- Dependencies: allowlist in R11 only. Adding one = stop, record in `DEVIATIONS.md`, await planner.

**Correctness discipline**
- No `unwrap`/`expect`/`panic!` outside tests, `main.rs` wiring, and `safety.rs`'s deliberate test panic. Clippy denies them (`clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic` — allowlisted per-file where required).
- `unsafe` only in `safety.rs`, each block carrying a `// SAFETY:` comment naming the async-signal-safety argument.
- Every state-changing `smc` write goes through write-verify (R1). Never update logical state from an unverified write (the `mbpfan.c:410` class).
- Temps are `MilliC`, never bare `i32` across module boundaries. rpm are `u32`.

**Style**
- `cargo fmt` default config, no customization. `//!` module doc on every file stating its contract and invariants in ≤10 lines. Public items get doc comments. Safety-critical lines get `// SAFETY:` or `// INVARIANT:` comments.
- Errors: per-module `thiserror` enum; error messages name the path/key and the fix. Logging via `tracing` macros only — no `println!` outside `cli.rs` output formatting.
- Tests: unit tests co-located (`#[cfg(test)]`); cross-module tests in `tests/`. Every behavioral claim in this PRD that is testable on fixtures/mocks **must** have a test (see R10 list).

**Safety of the build process itself (non-negotiable)**
- **Never write to the real `/sys` during development.** All tests run against `tests/fixtures/sysfs/` via `--sysfs-root`. Real-hardware tests run only behind `--features hw` + `AFANCTL_HWTEST=1` + applesmc present, and only in the final supervised gate (§9). If a test would write real sysfs without those guards, that test is a bug.
- Never run the daemon against real hardware as part of ordinary development; the supervised gate does that, with the user present.

**Cross-agent protocol**
- You may only create/modify the files listed in your task card. Everything else is read-only.
- Signatures come from Appendix A (repo copy: `DESIGN.md`). If a required change is discovered: implement everything else, record the proposed change in `DEVIATIONS.md` (old → new → why → which tasks are affected), and flag it in your final summary. Never silently rename/add public items.
- Ambiguity or a blocking question → write it to `QUESTIONS.md`, continue with the unambiguous remainder, and say so in your summary. Do not invent spec.
- LOC budget per file in §7; >20% over = stop and report (scope smell), do not absorb silently.

**Quality gates (every task, before declaring done)**
```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   # must SKIP cleanly (no applesmc in dev containers) — proves the guard works
```
Plus the task card's own checks. A task is done only when all gates pass in a clean checkout of its branch.

## Appendix P2 — Instruction template (render per task)

```
ROLE: You are implementing exactly ONE task of the afanctl project (Rust fan supervisor,
A1708 MacBook). You are part of a multi-agent build; other agents own other files.

READ FIRST (in order): DESIGN.md (contracts + conventions — BINDING), then this card.
Spec authority for your card: the PRD.md sections it cites (PRD.md sits at the repo root —
read the cited sections, not the whole PRD, unless you need depth). You may read, never
edit, any other repo file. orchestration/** is orchestrator-private: read-only to you.

YOUR TASK:
<card from PLAN.md §2, verbatim>

RULINGS SINCE THIS CARD WAS WRITTEN (binding): <only if any — §3.6 propagation>

CONVENTIONS (verbatim from DESIGN.md §8 = PLAN Appendix P1): <paste>

RULES:
- Touch ONLY the files listed in your card, plus append-only entries to DEVIATIONS.md /
  QUESTIONS.md when needed. Everything else is read-only.
- Use EXACTLY the signatures in DESIGN.md Appendix A. Need a change? Record it in
  DEVIATIONS.md (old → new → why → affected tasks), keep everything else working, flag it
  in your summary. Never silently alter a public item.
- Ambiguity? Write it to QUESTIONS.md, do the unambiguous rest, say so in your summary.
  Do not invent spec.
- NEVER write to the real /sys. All tests use tests/fixtures/sysfs via --sysfs-root.
- NEVER use sudo or pkexec, never install packages, never modify anything outside this
  worktree. rust/cargo are preinstalled. Do not merge; do not touch main.
- No new dependencies, no unsafe (unless your card says safety.rs), no unwrap/expect/panic
  outside tests, LOC budget: <per card>.
- COMMIT all work on the current branch and leave git status clean (untracked files fail
  verification).

DONE WHEN (run and show output of all): cargo fmt --check;
cargo clippy --all-targets --all-features -- -D warnings; cargo test;
cargo test --features hw  (must skip cleanly); plus your card's own checks.

FINAL OUTPUT: files changed, gate outputs, tests added, deviations/questions logged,
and anything you deliberately left for a later task.
```

## Appendix P3 — Review checklists

**Per merge (orchestrator, items 1–5 from PRD §11.5):**
1. Signatures match Appendix A (or an APPROVED DEVIATIONS entry exists and is consistent).
2. Every write path goes through verify; no logical state from unverified writes.
3. Every new failure path ends in AUTO / refusal-to-start / loud log — never silence.
4. Tests added cover the card's behaviors; no test writes real sysfs.
5. Conventions: fmt/clippy clean, no stray deps, error messages name key+fix.

**T9 adds item 6:** each mbpfan defect class (C1 C2 H1 H2 H3 H4 H5 H6) mapped to the test that proves it dead — plus the §9.1–9.5 evidence audit and README accuracy (full form per PRD §11.2-T9).

## Appendix P4 — Ledger format

```markdown
# afanctl orchestration ledger — orchestrator-owned; subagents read-only

## Index
| id | title | model | branch | status | dispatches | bounces | timeouts | merged (sha) |
|----|-------|-------|--------|--------|------------|---------|----------|--------------|
| T0 | scaffold+contracts | glm-5.3-flash/high | t0-scaffold | PENDING | 0 | 0 | 0 | — |
<!-- statuses: PENDING · DISPATCHED · VERIFYING · BOUNCED(n) · MERGED · BLOCKED · ESCALATED -->

## Events (append-only, newest last)
- [2026-09-14 15:02] T0 DISPATCHED (title "afanctl T0 scaffold", log orchestration/logs/T0-1.log)
- [2026-09-14 15:31] T0 VERIFIED: gates green, checklist 1–5 pass, ownership clean → MERGED <sha>
- [2026-09-14 16:10] RULING on QUESTIONS.md #[n] (raised by T2): <ruling> → propagated to T5 card
- [2026-09-14 16:40] DEVIATIONS T3-1 (panic_fd → Option<RawFd>): REJECTED — mock None already covers; T3 bounced

## Audits
### A1 (after T5, <date>) — <verbatim audit command outputs>
### A2 (after T8, <date>)  — <verbatim>

## Final report  <!-- written before handoff -->
```

## Appendix P5 — Supervised hardware gate handoff (USER-present; orchestrator prints and stops)

From PRD §9.3/§9.4 — the only real-sysfs session in the entire project. The orchestrator never runs these; it hands them over:

**Discipline (learned the hard way): after every package reinstall, restart the unit and confirm the loaded build.** `sudo systemctl restart afanctl`, then check that the journal's startup evidence line is fresh (`ExecMainStartTimestamp` after the package's install time). A fresh install without a restart silently keeps testing the old binary — that cost one hardware round. F19 adds a `doctor` WARN for exactly this.

- [x] a. `sudo afanctl doctor` — **PASS 2026-09-14** (7 PASS + 1 expected WARN: unit not installed). Do **not** run it as a plain user: the write-mode checks (`fan1_manual` writable, L2 fd armed) open the manual file `O_WRONLY`, so they FAIL by design without root (README "Root required (F11)"). `WARN` lines never block; the `systemd unit health — unit not installed` WARN is expected until `makepkg -si` in §9.4.
- [x] b. `sudo afanctl doctor --roundtrip` — **PASS 2026-09-14**: manual @ 1200 rpm verified over 2 s (observed 1210 rpm / Manual during the hold), AUTO restored. Non-root it refuses to write (no L2 fd) rather than writing manual — by design.
- [x] c. `sudo afanctl selftest-panic` — **PASS 2026-09-14**: deliberate panic at `src/safety.rs:81`, exit 101; `fan1_manual` reads `0` afterwards.
- [x] d. `sudo systemctl start afanctl` (observe) — **PASS 2026-09-14** (soak): the unit ran **1 h 10 min** continuously with `Restart=always`, consumed **1.966 s CPU over 4211 s wall (0.05 %)** and peaked at **2.4 MB** RSS — R11's <0.1 % CPU and <5 MB budget both met, no errors in the journal apart from the deliberate curve experiments. Observed via `systemctl status` + `status --json` reading the state file as a normal user.
- [x] e. `SIGKILL` the daemon in curve mode — **PASS 2026-09-14 21:48** (the acceptance test that had failed three times): `sudo afanctl curve` ⇒ `fan1_manual == 1` within 3 s (first time on hardware), `status` showed `mode: curve`, `manual=true`, `target: 1200 rpm`, fan 2005 rpm converging, `recent_errors: none`; `systemctl kill -s SIGKILL` ⇒ restart at 21:48:46 with the journal showing `startup reconcile: previous process died without restoring AUTO (fan1_manual=1; SIGKILL/OOM-kill cannot run L2); restoring AUTO now` then `startup reconcile: AUTO restored (fan1_manual=0, verified)`, and `fan1_manual` read `0`. `doctor` also PASSed its new stale-binary check (`running daemon matches installed binary`). Note: because `/run/afanctl/cmd.json` holds a standing `curve` command (R8, by design), the restarted daemon re-applies it within a poll, so `manual=1` returns; `sudo afanctl observe` releases the fan.
- [x] f. curve soak + `doctor --compare 600` — **PASS 2026-09-14** (ran clean: no errors, no fallback, `status` showed `mode: curve`, `target: 1200 rpm`, `manual=true`; at `t_eff` 63 °C — below `high = 66` — our curve held 1200 rpm, fan 1174). **Empirical verdict from the 600-sample comparison: ours does not beat the incumbent at idle** — mean Δ 191 rpm, max |Δ| 2260, ours quieter in 198 / louder in 392 / equal in 10. Read by regime: at `t_eff` ≤ 64 °C ours ≈ the SMC within ±20 rpm (both ~1190–1220); during short coretemp spikes (66–85 °C) ours ramps toward the linear target at the 750 rpm/poll slew while the SMC stayed near 1.2k rpm (its own sensors did not register those spikes) — hence the loud-side deltas up to ~2.2k rpm. So the honest characterisation: ours is **more responsive to coretemp and verifiable**, not quieter. Knobs if quiet is the goal: raise `[thresholds].high` (start ramping later) and/or lower `[curve].max_rpm`.
- [x] g. `hold 3000` via the polkit rule — **PASS 2026-09-14**: `pkexec afanctl hold 3000` (passwordless rule worked, no prompt), `status` → `mode: hold`, `target: 3000 rpm`, fan **2980 rpm**, `manual=true`, `recent_errors: none`; `sudo afanctl observe` released it. **Two corrections to this step as originally written:** (i) `once --at-temp 86 --dry-run` prints `SetSpeed(1950)`, not `EscalateMax` — the decision carries the slew-limited step (750 rpm from the previous command) and the overshoot guard requires `OVERSHOOT_POLLS = 3` **consecutive polls inside one process**, while `once` is stateless, so the guard is unreachable from the CLI and is proven by the policy trace tests (PRD §6.R10) instead; (ii) `doctor` did not warn "hold active" — that check simply did not exist (ticket F25 adds it). PRD §9.3g corrected accordingly.
- [x] `makepkg -si` builds and installs cleanly; `systemctl enable` survives a reboot test (boots in observe, fan on the SMC curve). **Ready to verify: install done (02:02 build, doctor 10/10 PASS) and `systemctl enable` done (2026-09-15 02:13) — the reboot itself is the user's, since it ends the session; expected result is in the ledger's final pass entry.**

---

## Orchestration risks (this plan's own risks; product risks live in PRD §10)

| Risk | Mitigation |
|---|---|
| `--auto` lets an agent run destructive commands | RULES block forbids sudo/pkexec/install/outside-worktree; §3.5 ownership diff + HALT conditions; verification is mechanical |
| Parallel worktrees drift / merge conflicts | Branches cut from current main (deps merged); shared files append-only; rebase before merge; gates re-run post-rebase and on merged main |
| DEVIATIONS storm breaks the frozen contracts | Reject-by-default adjudication (§3.6); accept requires DESIGN.md amendment + in-flight re-dispatch |
| opencode session hangs on permissions mid-task | S0 write-smoke with `--auto` validates the path before real dispatches |
| Subagent self-reports diverge from reality | §3.5: gates re-run by the orchestrator, `git status --porcelain` empty, ownership diff — self-reports are never evidence |
| Agent leaves uncommitted/untracked files faking green gates | Clean-status requirement in template RULES + verify-time check |
| Provider outage mid-wave | Model-outage policy (§3.8): retry once, mark BLOCKED, tell the user |
| Plan/PRD divergence over a long build | PRD wins (stated in header); PLAN.md amendments are ledger-noted orchestrator actions |

## Post-gate (out of scope for this plan — PRD §12)

Publication (after Q1 name re-check on AUR + crates.io), omafan plugin build on R8, `auto-intervene` mode, multi-fan, man page — all deferred by the PRD; do not build, do not plan.
