# T9c — adversarial review gate (round 3): F19 + F20, and an explicit attempt to falsify RULING F20's own criterion

**READ-ONLY: fix nothing.** You may read files, run the gates and fixture tests, and probe with the real
binary against **tempdir copies** of `tests/fixtures/sysfs/`. Never write to real `/sys`, never run
`--features hw` against hardware, never interfere with the installed system daemon (read-only
observation of it is fine).

Read `DESIGN.md` (BINDING), `orchestration/LEDGER.md` (the rulings' evidence trail) and
`orchestration/REVIEW-T9b.md` (round 2) first. `PRD.md` for requirements.

## Subject

Everything merged after the T9b review (`b0fba6e`):

- **F19** — settle-window verification (`ECHO_SETTLE_MS = 1500 / ECHO_SETTLE_SAMPLES = 10`,
  `MODE_SETTLE_MS = 1000`, `WRITE_RETRY_MAX = 1`), the µs retry ladder's removal,
  `MockSmc::set_echo_latency` + `tests/settle_window.rs`, `doctor`'s stale-binary check.
- **F20** — stall detector re-keyed to **tach movement** (`STALL_TACH_EPSILON_RPM = 50`), the honest and
  self-recovering failed-AUTO path (`auto_restore_pending`, every-poll retry, rate-limited log, additive
  state/status field + human marker), the `l2_absent` guard on the cmd-file channel, off-target dwell
  visibility (`OFF_TARGET_WARN_POLLS = 30`), `tests/pin_constants.rs`, README wording.
- The DESIGN.md/PRD wording changes those rulings produced.

## Mandate (in priority order)

1. **Falsify RULING F20 R1 — do this first.** The stall criterion is "the tach moved more than
   `STALL_TACH_EPSILON_RPM` since the previous poll". Attack it:
   - a fan **jittering** ±60–100 rpm while parked far from the command (real fan bearing noise, or an
     obstruction) — does the detector ever fire, or does jitter count as "progress" forever?
   - a fan whose command the **SMC clamps** (e.g. a firmware floor above our target): tach stable, no
     progress, detector fires and degrades — false positive? How likely, and is degrading the right
     answer or is it noise that will page the user for a healthy fan?
   - a fan moving *away* from the command while the command ramps faster than the fan can follow
     (curve slew 750 rpm/poll vs measured ~3000 rpm/s): does a healthy fan ever get declared stalled?
   Give a verdict per case, with a reproduction where you can produce one. If R1 is falsifiable, say so
   plainly and propose the minimal criterion that survives all four (movement *toward* the command vs
   movement *at all*; net deviation over the window; etc.).
2. **F19's settle window — attack the false-PASS direction.** A window that accepts a stale register
   value could accept a write the hardware *rejected* (e.g. the register already held the target value
   from a previous command → instant "match" with no proof the new write took, or the SMC rejecting a
   target it clamps to a *different* value). Is there a target for which the current check reports
   success while the fan is not actually commanded there? Also: what does the window cost — worst-case
   blocking inside one poll (up to ~3 s) versus `WatchdogSec=15`, the poll cadence, the overshoot
   guard's 3-poll escalation, and the plugin's cmd-file latency. Prove or refute from the code.
3. **AUTO-restore recovery (F20 R2).** Can `auto_restore_pending` end up latched forever while the
   process looks healthy? Is the retry genuinely every poll? Does anything re-arm speed control after a
   successful late restore (or does the daemon sit in monitor-only while `auto_restore_pending` clears)?
   Is the flag's human/JSON rendering accurate? Any way to report success without a verified read-back?
4. **Does the fix class hold?** Both F19 and F20 were built from *one* hardware observation each. Look
   for other places where product code assumes the mock's instant/synchronous/polite behaviour. Name
   them even if you cannot prove a defect (mark speculative).
5. **Regression risk in what was touched.** 152 tests pass; identify tests that would *still* pass if
   the fix were reverted (kill-analysis, as T9b did) — those are the weak spots.
6. **Contract hygiene.** Appendix A/B vs code (constants list, additive fields, schema ids), the frozen
   signatures, `unsafe` confinement, dependency allowlist, LOC budgets (±20%), no
   `unwrap/expect/panic!` in product code, docs vs behaviour.

## Output (commit as `orchestration/REVIEW-T9c.md` on your branch)

- **Verdict:** PASS, or a prioritized list: `SEVERITY · file:line · what is wrong · why it matters ·
  reproduction · suggested direction`.
- A dedicated section **"RULING F20 R1: falsified / survives"** with the four attack cases answered
  concretely and an explicit recommendation (keep as ruled, or the exact amended criterion).
- Separate speculative items from proven ones. Every claim gets a file:line; unproven claims are
  labelled **unproven**. A short honest list beats a padded one.

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
