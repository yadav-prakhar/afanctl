# T9b — adversarial review gate (round 2): everything merged after the T9 gate

You are the **adversarial reviewer** for the afanctl project. **READ-ONLY: fix nothing.** You may read
any file, run the gates and the fixture test suite, and inspect git history. You must **not** write to
real `/sys`, must not run the daemon against real hardware, and must not run `--features hw` tests
against hardware (the guard makes them skip; that is expected).

Read `DESIGN.md` (BINDING) and `PRD.md` first, then `orchestration/LEDGER.md` — the ledger records four
rulings (F14, F16, F18, N-F14-1) and the hardware evidence behind them. Your subject is **the code that
landed after the first review gate (`T9`)**:

- `F14` — startup reconcile (`run()` now: arm L2 → reconcile → evidence line → READY → loop)
- `F16` — register-echo write verification, L1 mode-drift/tracking split, `STALL_POLLS` stall detector,
  `MockSmc` tach-lag / frozen / write-stuck simulation, rewritten fixture-verify test
- `F18` — additive `monitor_only` in `state.v1` + `status.v1`, human `status` completeness, smc pre-open
  WARN → debug, `RuntimePaths.config_source`
- the four PRD edits those rulings produced

## Why this review exists

The supervised hardware gate found a **critical defect class that the first review gate and 126 tests
could not see**: the code verified a fan *command* against the fan's *tachometer*, and the mock/fixture
backends echo a write instantly, so the tests encoded idealized physics rather than reality. Two other
defects were in the same neighbourhood (no startup reconcile for uncatchable deaths; the degraded latch
invisible to callers). Your mandate is to hunt that class deliberately, not just the diff's local
correctness.

## What to attack (in priority order)

1. **Fixture-shaped assumptions.** Any place where product code's correctness depends on behavior that
   only the mock or the fixture tree exhibits. Read every mock setter and ask "what does the real
   applesmc do here instead?". Specifically re-examine: writes while the SMC is still in `Auto`
   (does the SMC ignore them? does `fan1_output` then read the SMC's own target?), `fan1_output`
   clamping/rounding by firmware, `fan1_input` semantics, `fan1_manual` write latency, whether a
   `fan1_output` echo can be stale or cached, and whether any read is assumed atomic.
2. **Ruling compliance.** For each of F14/F16/F18: does the implementation match the frozen ruling
   exactly? Do the tests actually test the ruling, or do they pass for an unrelated reason (e.g. a test
   asserting a field that would be the same under the buggy behavior)? Try to name a concrete change
   that would make each test fail.
3. **The remaining safety envelope.** With F16's split, what failure can now go *unnoticed* that the old
   (buggy) code would have caught? Specifically: can a fan be commanded but never move without the stall
   detector firing (e.g. slow drift, oscillation, intermittent write failures, `monitor_only` set while
   `manual_armed`)? Is `WRITE_ECHO_TOLERANCE_RPM = 50` / `STALL_POLLS = 10` defensible against real
   firmware rounding and 1 Hz polling?
4. **State-machine corners.** Reconcile + cmd-file + monitor-only + hold interactions: reconcile at
   startup while a `hold` command is already present; `monitor_only` re-arm semantics (R4: only a
   *changed* command re-arms — is that still right for the plugin?); observe→curve→hold transitions;
   panic during reconcile; a `fan1_manual=1` left by a *foreign* program.
5. **Contract hygiene.** Appendix A/B in `DESIGN.md` vs the code: every public item present and
   unchanged except the ruled ones; `monitor_only` genuinely additive (a v1-schema consumer that ignores
   it must keep working); no `unsafe` outside `safety.rs`; dependency allowlist; per-file LOC budgets
   (±20%); no `unwrap`/`expect`/`panic!` in product code.
6. **Docs vs behavior.** README/`-h` claims that the code does not satisfy. The hardware gate already
   caught one (status fields promised but not printed) — look for the rest.

## Output (commit as `orchestration/REVIEW-T9b.md` on your branch)

- **Verdict:** PASS, or a prioritized list — for each: `SEVERITY (CRITICAL/MAJOR/MINOR) · file:line ·
  what is wrong · why it matters · reproduction (test name / exact command) · suggested fix direction`.
- Separate section: **"assumptions that only hold on fixtures"** — your list, even where you could not
  prove a defect (mark speculative ones as such, clearly separated from proven ones).
- Every claim must be backed by a file:line citation; a claim you could not reproduce must be labelled
  **unproven**. Do not inflate: a short honest list beats a padded one.

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
