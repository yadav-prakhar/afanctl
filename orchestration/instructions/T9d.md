# T9d — adversarial review gate (round 4, final): F21 + F22, and a close/no-close judgement on the review gate

**READ-ONLY: fix nothing.** You may read files, run the four gates and the fixture suites, and probe with
the real binary against **tempdir copies** of `tests/fixtures/sysfs/`. Never write to real `/sys`, never
actuate hardware (`--features hw` must skip), never disturb the installed daemon (read-only observation is
fine).

Read `DESIGN.md` (BINDING), `orchestration/LEDGER.md`, `orchestration/REVIEW-T9b.md` and
`orchestration/REVIEW-T9c.md` first. This is the last planned review round; your verdict decides whether
the review gate closes.

## Subject

Everything merged after the T9c review (`b0fba6e`-descendants):

- **F21** — observe-release ownership fix (`apply_mode`'s observe branch keeps `manual_armed`, sets
  `auto_restore_pending`, never claims an unverified release; the every-poll retry completes it); the
  watchdog ping moved to **both ends** of `step_once`; `config::MAX_INTERVAL_S` 14 → **12**; the off-target
  dwell WARN now **repeats** every `OFF_TARGET_WARN_POLLS`; the F19/F20 constants and both cadences pinned
  by value *and* behaviour (including the jitter trade-off pair: sub-epsilon jitter stalls,
  super-epsilon jitter never stalls but warns repeatedly).
- **F22** — additive `polls` counter in `afanctl.state.v1`; `daemon.uptime_s` = `polls × interval_s`
  (documented fallback to the old ping-based estimate for a state file from an older build);
  `watchdog_pings` keeps its two-per-poll meaning; the test that pinned the wrong arithmetic corrected.

## Mandate

1. **Falsify the fixes.** For each of F21 R1 (observe release), F21 R2 (ping cadence + cap 12), F21 R3
   (repeating dwell WARN), F22 (`polls` / `uptime_s`): name the change that would make its test fail, and
   try to reach the failure by another route. In particular:
   - can the fan still end up in Manual, unsupervised, with a healthy-looking state, through *any* channel
     (observe cmd, fallback, reconcile, hold, monitor-only re-arm, a foreign writer)? Trace every path that
     clears `manual_armed`.
   - does the two-ping scheme actually bound the gap the reviewer computed in T9c (worst-case poll work
     ≈2.7 s at `interval_s = 12` vs `WatchdogSec = 15`)? Recompute, including the settle-window cost and
     any path where a poll can block longer than the ruling assumed.
   - `uptime_s` with `interval_s = 12` and a long-running daemon: exact? And is the fallback path reachable
     with a *current* state file (i.e. can `polls` be missing while `watchdog_pings` is present)?
2. **Regression surface.** Re-run the kill-analysis style check on F21/F22 and confirm nothing from the
   F16/F19/F20 semantics was disturbed (the settle window, the echo tolerance, the tracking/drift split,
   the stall criterion, the reconcile).
3. **Anything still fixture-shaped.** This is the last chance: list every remaining assumption in product
   code that only holds on the mock/fixture physics, with a severity judgement — and say plainly which of
   them the hardware gate can and cannot cover, so the handoff is honest.
4. **Contract hygiene:** Appendix A/B vs code (all ruled constants present — the list was just completed;
   the additive fields; unchanged schema ids and signatures), `unsafe` confinement, dependency allowlist,
   LOC budgets, no `unwrap/expect/panic!` in product code, docs vs behaviour.
5. **Gate judgement:** end with an explicit **"review gate: CLOSE / DO-NOT-CLOSE"** recommendation and, if
   DO-NOT-CLOSE, the minimum set of fixes that would change your mind. Be honest — a FAIL with two real
   findings is worth more than a PASS that hides one.

## Output (commit as `orchestration/REVIEW-T9d.md` on your branch)

- **Verdict:** PASS / FAIL, then the prioritized findings (`SEVERITY · file:line · what · why · reproduction ·
  direction`), then the speculative "only holds on fixtures" list (labels preserved), then the gate
  judgement. Every claim gets a file:line; unproven claims are labelled **unproven**.

Constraints: no source edits (the report file is the only write), no new dependencies, commit on your
branch, `git status` clean, do not merge, do not touch `main`.

---

## CONVENTIONS (verbatim from DESIGN.md §8 — binding)

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
