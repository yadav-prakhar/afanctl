ROLE: You are fixing ONE defect in the afanctl project (Rust fan supervisor, A1708 MacBook),
as part of a multi-agent build. This is a FIX TICKET (bounce of T4).

READ FIRST: DESIGN.md (BINDING) → this ticket → PRD §6.R4/§7 Appendix A notify section.

DEFECT (verified by the orchestrator):
`cargo test --lib` flakes 10-30%% under default parallel test threads:
test notify::tests::abstract_socket_form_reaches_listener ... FAILED
assertion failed: sd_watchdog() at src/notify.rs:109
Root cause: the notify tests mutate the SHARED process environment (std::env::set_var /
remove_var for NOTIFY_SOCKET) and run in parallel threads — tests race, one thread's
remove_var/set_var yanks the socket path out from under another. With
--test-threads=1 everything passes deterministically (verified 5x).

FIX REQUIREMENTS:
1. Make the notify tests deterministic under parallel execution. Two acceptable shapes:
   (a) spawn each env-dependent test's body inside a single dedicated thread while holding a
       shared mutex acquired before set_var and released after the final assertion — env
       mutations serialized, test threads block on the mutex; or
   (b) refactor production code so the socket path is a parameter (NOT process-global) and
       have the tests pass the path explicitly — but this changes public signatures, so if
       DESIGN.md's notify::sd_* signatures lack the parameter you MUST record it in
       DEVIATIONS.md as a proposal instead of changing them, and use shape (a).
   Note std::env::set_var is unsafe as of edition 2024 — this crate is edition 2021, so
   direct calls are fine, but serialization is mandatory for correctness, not style.
2. Touch ONLY src/notify.rs (production code + tests). Fix NOTHING else even if you notice
   other issues — report those in your final output instead.
3. The tests must still assert exactly what they assert now (READY/WATCHDOG/STATUS bytes on
   the wire, dead-socket false). Do not weaken assertions to dodge the race.

RULES (binding):
## 8. Engineering conventions (BINDING — paste verbatim into every subagent instruction)

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

- No new dependencies, no unsafe, do not merge, no real /sys, COMMIT on current branch, leave
  git status clean.

DONE WHEN (show outputs):
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test                # run this FIVE consecutive times; all must pass deterministically
cargo test --features hw  # must SKIP cleanly

FINAL OUTPUT: the chosen fix shape, the five consecutive cargo test results, files changed.
